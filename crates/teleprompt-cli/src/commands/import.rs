use std::path::PathBuf;

use teleprompt::draft::document;
use teleprompt::draft::import::*;
use teleprompt::setup;

use crate::cli::{emit, runtime_failure, Run};
use crate::output::{Format, Outcome};

/// What `import` reads, and where it writes. A session recording takes
/// the session flags; anything else, the document flags.
#[derive(clap::Args)]
pub struct Args {
    /// What to draft from: a Markdown document, a Slidev deck, a
    /// conversation's captions, transcript or recording, or a session you
    /// recorded while talking (an asciicast, a `vhs record` tape)
    pub source: PathBuf,
    /// Where to write the script; defaults to <doc>.teleprompt.md, or for a
    /// session scripts/<recording>.md in the project
    #[arg(long)]
    pub out: Option<PathBuf>,

    /// A document: read it as a Slidev deck, whose speaker notes become
    /// the narration, a paragraph per `[click]` step
    #[arg(long, conflicts_with = "transcript", help_heading = "A document")]
    pub slidev: bool,
    /// A document: read it as the transcript of a conversation, each turn
    /// a line opening with who says it, `**Ada:**`, and everyone in it the
    /// cast. Captions (.vtt, .srt) are read as one without it
    #[arg(long, help_heading = "A document")]
    pub transcript: bool,
    /// A conversation's recording (WAV, or anything ffmpeg reads): each
    /// line's stretch of it becomes its take, so it is spoken in its
    /// speaker's own voice until reworded. Draft into a project
    #[arg(long, value_name = "RECORDING", help_heading = "A document")]
    pub audio: Option<PathBuf>,
    /// Leave this speaker's lines to their voice in the cast rather than
    /// the recording; may be repeated
    #[arg(long, value_name = "SPEAKER", help_heading = "A document")]
    pub revoice: Vec<String>,
    /// How many people speak in a recording, where you know: it tells
    /// their voices apart better than guessing
    #[arg(long, value_name = "N", help_heading = "A document")]
    pub speakers: Option<usize>,

    /// A session: the voice recorded with it, as a WAV
    #[arg(long, help_heading = "A recorded session")]
    pub voice: Option<PathBuf>,
    /// The tool that made the session, by its scene plugin; found by its
    /// extension otherwise
    #[arg(long, help_heading = "A recorded session")]
    pub with: Option<String>,
    /// Directory of an unpacked sherpa-onnx streaming zipformer model;
    /// defaults to the one `teleprompt setup speech-model` installed
    #[arg(long, help_heading = "A recorded session")]
    pub model: Option<PathBuf>,
    /// Timed words as JSON ([{text, start_ms, end_ms}]) instead of --model
    #[arg(long, conflicts_with = "model", help_heading = "A recorded session")]
    pub words: Option<PathBuf>,
    /// How many milliseconds after the session the voice recording started
    #[arg(
        long,
        default_value_t = 0,
        allow_negative_numbers = true,
        help_heading = "A recorded session"
    )]
    pub offset_ms: i64,
    /// Directory of an unpacked sherpa-onnx punctuation model, to give the
    /// narration capitals and punctuation
    #[arg(long, help_heading = "A recorded session")]
    pub punctuation: Option<PathBuf>,
    /// Replace the script, its recording and its lines' takes, if they
    /// exist
    #[arg(long, help_heading = "A recorded session")]
    pub force: bool,
}

impl Args {
    /// Whether `source` is a session a recorder reads, rather than a
    /// document: named as one, given its voice, or by its extension.
    fn is_session(&self) -> bool {
        self.voice.is_some() || self.with.is_some() || recorder_for(None, &self.source).is_ok()
    }

    /// The flags given that are for the other kind of source.
    fn misplaced(&self, session: bool) -> Vec<&'static str> {
        let document = [
            ("--slidev", self.slidev),
            ("--transcript", self.transcript),
            ("--audio", self.audio.is_some()),
            ("--revoice", !self.revoice.is_empty()),
            ("--speakers", self.speakers.is_some()),
        ];
        let recorded = [
            ("--model", self.model.is_some()),
            ("--words", self.words.is_some()),
            ("--offset-ms", self.offset_ms != 0),
            ("--punctuation", self.punctuation.is_some()),
            ("--force", self.force),
        ];
        let other = if session { &document } else { &recorded };
        other
            .iter()
            .filter(|(_, given)| *given)
            .map(|(f, _)| *f)
            .collect()
    }
}

/// `import`: a recorded session, or anything else as a document.
pub fn run(args: Args, format: Format) -> Run {
    let session = args.is_session();
    let misplaced = args.misplaced(session);
    if !misplaced.is_empty() {
        let kind = if session {
            "a document or conversation, and this is a recorded session"
        } else {
            "a recorded session, which is given with --voice"
        };
        return Err(Outcome::ValidationError(vec![format!(
            "{} {} for {kind}",
            misplaced.join(", "),
            if misplaced.len() == 1 { "is" } else { "are" },
        )]));
    }
    if session {
        run_session_import(format, args)
    } else {
        run_document_import(format, args)
    }
}

fn run_session_import(format: Format, args: Args) -> Run {
    let Some(voice) = args.voice.as_deref() else {
        return Err(Outcome::ValidationError(vec![format!(
            "{} is a recorded session: give the voice recorded with it, --voice <wav>",
            args.source.display()
        )]));
    };
    let script = match args.out.clone() {
        Some(out) => out,
        None => default_import_script(&args.source)?,
    };
    let model;
    let words = match &args.words {
        Some(path) => Words::File(path),
        None => {
            model = setup::speech_model(args.model.as_deref()).map_err(runtime_failure)?;
            Words::Model(&model)
        }
    };
    let punctuation = setup::punctuation_model(args.punctuation.as_deref());
    let report = run_import(&Import {
        recording: &args.source,
        with: args.with.as_deref(),
        voice,
        script: &script,
        words,
        offset_ms: args.offset_ms,
        punctuation: punctuation.as_deref(),
        force: args.force,
    })
    .map_err(Outcome::RuntimeFailure)?;
    emit(format, &report, &report.render());
    Ok(Outcome::Ok)
}

/// `scripts/<recording>.md` in the project around the working directory.
fn default_import_script(recording: &std::path::Path) -> Result<PathBuf, Outcome> {
    let project = crate::cli::project_here()?;
    let stem = recording.file_stem().unwrap_or_default().to_string_lossy();
    Ok(project.root.join("scripts").join(format!("{stem}.md")))
}

fn run_document_import(format: Format, args: Args) -> Run {
    let reading = match (args.slidev, args.transcript) {
        (true, _) => document::Reading::Slidev,
        (_, true) => document::Reading::Transcript,
        _ => document::Reading::Document,
    };
    let audio = document::Audio {
        path: args.audio.as_deref(),
        revoice: &args.revoice,
        speakers: args.speakers,
    };
    let report =
        document::run_document(&args.source, args.out, reading, audio).map_err(runtime_failure)?;
    emit(format, &report, &report.render());
    Ok(Outcome::Ok)
}
