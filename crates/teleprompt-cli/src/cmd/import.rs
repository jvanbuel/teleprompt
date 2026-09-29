//! `teleprompt import <recording> --voice <wav>`: a session recorded with
//! an adapter's tool (a cast, a tape) and the voice recorded with it
//! become a script, and the takes its lines are spoken from. The deriving
//! is `teleprompt-derive`; reading the recording is its adapter's; this
//! hears the voice and writes the results.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use teleprompt_capture::record::{Recorded, Recorder};
use teleprompt_derive::{derive, Draft, Line, Options, Word};
use teleprompt_voice::takes::Takes;
use teleprompt_voice::{wav, Pcm};

use crate::cmd::check::compile_script;
use crate::project::Project;

/// Where the words come from.
pub enum Words<'a> {
    /// A sherpa-onnx streaming model's directory: the voice is transcribed.
    Model(&'a Path),
    /// A JSON list of `{text, start_ms, end_ms}`, from another recognizer
    /// or a corrected transcript.
    File(&'a Path),
}

pub struct Import<'a> {
    pub recording: &'a Path,
    /// The adapter whose tool made it; found by its extension when `None`.
    pub with: Option<&'a str>,
    pub voice: &'a Path,
    pub script: &'a Path,
    pub words: Words<'a>,
    /// How long after the recording started the voice recording did.
    pub offset_ms: i64,
    /// A sherpa-onnx punctuation model's directory: the words are
    /// punctuated. Without one, each line is only capitalized and stopped.
    pub punctuation: Option<&'a Path>,
    /// Overwrite the script, its recording and its lines' takes.
    pub force: bool,
}

#[derive(Debug, Serialize)]
pub struct ImportReport {
    pub created: PathBuf,
    /// The recording the script's blocks include, marked into parts.
    pub recording: PathBuf,
    pub lines: usize,
    pub blocks: usize,
    /// The lines now spoken from the recording, by id.
    pub takes: Vec<String>,
}

impl ImportReport {
    pub fn render(&self) -> String {
        format!(
            "drafted {} from the session: {} line(s), {} block(s) of {}, {} take(s) recorded\n  \
             the prose is what was said, verbatim: edit it, then record the lines you \
             change again with `teleprompt prompt`\n",
            self.created.display(),
            self.lines,
            self.blocks,
            self.recording.display(),
            self.takes.len()
        )
    }
}

/// How much of the recording a take keeps around its words.
const LEAD_MS: u64 = 150;
const TAIL_MS: u64 = 250;

pub fn run_import(imp: &Import) -> Result<ImportReport, String> {
    refuse_to_replace(imp.script, imp.force)?;
    let recorder = recorder_for(imp.with, imp.recording)?;
    let text = std::fs::read_to_string(imp.recording)
        .map_err(|e| format!("cannot read {}: {e}", imp.recording.display()))?;
    let recorded = recorder
        .read(&text)
        .map_err(|e| format!("{}: {e}", imp.recording.display()))?;
    draft_session(&Session {
        script: imp.script,
        recorder: &*recorder,
        recorded: &recorded,
        voice: imp.voice,
        words: &imp.words,
        offset_ms: imp.offset_ms,
        punctuation: imp.punctuation,
        force: imp.force,
    })
}

/// The recorder `with` names, or the one whose recordings have
/// `recording`'s extension.
pub fn recorder_for(with: Option<&str>, recording: &Path) -> Result<Box<dyn Recorder>, String> {
    let all = crate::scene::recorders();
    let names: Vec<&str> = all.iter().map(|r| r.adapter()).collect();
    let names = names.join(", ");
    let ext = recording.extension().and_then(|e| e.to_str()).unwrap_or("");
    let found = all.into_iter().find(|r| match with {
        Some(name) => r.adapter() == name,
        None => r.extension() == ext,
    });
    found.ok_or_else(|| match with {
        Some(name) => format!("`{name}` cannot record a session; these can: {names}"),
        None => format!(
            "cannot tell which tool made {}; name it with --with ({names})",
            recording.display()
        ),
    })
}

/// A script about to be drafted over an existing one without `--force`.
pub fn refuse_to_replace(script: &Path, force: bool) -> Result<(), String> {
    if script.exists() && !force {
        return Err(format!(
            "{} already exists; pass --force to replace it and its lines' takes",
            script.display()
        ));
    }
    Ok(())
}

/// A recorded session, and the voice recorded beside it.
pub struct Session<'a> {
    pub script: &'a Path,
    pub recorder: &'a dyn Recorder,
    pub recorded: &'a Recorded,
    pub voice: &'a Path,
    pub words: &'a Words<'a>,
    /// How long after the recording's clock started the voice did.
    pub offset_ms: i64,
    pub punctuation: Option<&'a Path>,
    pub force: bool,
}

/// Drafts `session.script`: the recording saved beside it in parts, the
/// script including each part after the line it goes with, and each line
/// spoken from the voice.
pub fn draft_session(session: &Session) -> Result<ImportReport, String> {
    let project = Project::for_script(session.script).map_err(|e| e.to_string())?;
    let pcm = read_voice(session.voice)?;
    let words = hear(session.words, &pcm, session.offset_ms)?;
    if words.is_empty() {
        return Err(format!("nothing was heard in {}", session.voice.display()));
    }
    let words = match session.punctuation {
        Some(dir) => punctuated(dir, &words)?,
        None => words,
    };

    let stem = session
        .script
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    let include = format!("recordings/{stem}.{}", session.recorder.extension());
    let recording = crate::project::script_dir(session.script).join(&include);
    if recording.exists() && !session.force {
        return Err(format!(
            "{} already exists; pass --force to replace it",
            recording.display()
        ));
    }
    let options = Options {
        title: title_of(session.script),
        scene: session.recorder.adapter().to_string(),
        include,
        ..Options::default()
    };
    let starts: Vec<u64> = session.recorded.steps.iter().map(|s| s.start_ms).collect();
    let draft = derive(&starts, &words, &options);
    write(&recording, &session.recorded.marked(&draft.cuts()))?;
    write(session.script, &draft.markdown(&options))?;

    // The script as the compiler reads it, which is what names its lines.
    let (compiled, _) = compile_script(
        &project,
        session.script,
        &crate::cmd::check::source_locale(&project),
    )
    .map_err(|errors| {
        format!(
            "the drafted {} does not compile, which is a bug in `import`:\n{}",
            session.script.display(),
            errors.join("\n")
        )
    })?;
    let lines: Vec<(String, String)> = compiled
        .narration
        .iter()
        .map(|n| (n.line_id.clone(), n.text.clone()))
        .collect();
    let takes = save_takes(&project, &draft, &lines, &pcm, session.offset_ms)?;
    Ok(ImportReport {
        created: session.script.to_path_buf(),
        recording,
        lines: lines.len(),
        blocks: draft.blocks().count(),
        takes,
    })
}

/// Writes `text` to `path` whole or not at all, making its directory.
fn write(path: &Path, text: &str) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("cannot write {}: {e}", path.display());
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(fail)?;
    }
    let mut partial = path.as_os_str().to_owned();
    partial.push(".partial");
    std::fs::write(&partial, text)
        .and_then(|()| std::fs::rename(&partial, path))
        .map_err(fail)
}

fn read_voice(path: &Path) -> Result<Pcm, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    wav::decode(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// The recording's first channel, as floats.
fn mono(pcm: &Pcm) -> Vec<f32> {
    pcm.samples
        .iter()
        .step_by(usize::from(pcm.channels.max(1)))
        .map(|&s| f32::from(s) / 32768.0)
        .collect()
}

#[derive(Deserialize)]
struct WordEntry {
    text: String,
    start_ms: u64,
    end_ms: u64,
}

/// The words in the voice, on the recording's clock.
fn hear(words: &Words, pcm: &Pcm, offset_ms: i64) -> Result<Vec<Word>, String> {
    let heard = match words {
        Words::File(path) => {
            let bytes =
                std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            serde_json::from_slice::<Vec<WordEntry>>(&bytes)
                .map_err(|e| format!("{}: {e}", path.display()))?
                .into_iter()
                .map(|w| (w.text, w.start_ms, w.end_ms))
                .collect()
        }
        Words::Model(dir) => transcribe(dir, pcm)?,
    };
    let shift = |ms: u64| ms.saturating_add_signed(offset_ms);
    Ok(heard
        .into_iter()
        .map(|(text, start, end)| Word {
            text,
            start_ms: shift(start),
            end_ms: shift(end),
        })
        .collect())
}

#[cfg(feature = "listen")]
fn transcribe(dir: &Path, pcm: &Pcm) -> Result<Vec<(String, u64, u64)>, String> {
    use teleprompt_listen_sherpa::SAMPLE_RATE;
    let samples = teleprompt_voice::resample(&mono(pcm), pcm.sample_rate, SAMPLE_RATE);
    Ok(teleprompt_listen_sherpa::transcribe(dir, &samples)?
        .into_iter()
        .map(|w| (w.text, w.start_ms, w.end_ms))
        .collect())
}

/// `words` with the punctuation model's capitals and punctuation.
#[cfg(feature = "listen")]
fn punctuated(dir: &Path, words: &[Word]) -> Result<Vec<Word>, String> {
    let text: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
    let text = teleprompt_listen_sherpa::punctuate(dir, &text.join(" "))?;
    Ok(teleprompt_derive::punctuate(words, &text))
}

#[cfg(not(feature = "listen"))]
fn punctuated(_: &Path, _: &[Word]) -> Result<Vec<Word>, String> {
    Err(
        "this teleprompt was built without speech models: rebuild it with \
         `--features listen` to use --punctuation"
            .to_string(),
    )
}

#[cfg(not(feature = "listen"))]
fn transcribe(_: &Path, _: &Pcm) -> Result<Vec<(String, u64, u64)>, String> {
    Err(
        "this teleprompt was built without a speech recognizer: rebuild it with \
         `--features listen`, or pass --words"
            .to_string(),
    )
}

/// Saves each line's stretch of the recording as its take: its words, a
/// little either side, and never past halfway to the next line.
fn save_takes(
    project: &Project,
    draft: &Draft,
    lines: &[(String, String)],
    pcm: &Pcm,
    offset_ms: i64,
) -> Result<Vec<String>, String> {
    let spoken: Vec<&Line> = draft.lines().collect();
    if spoken.len() != lines.len() {
        return Err(format!(
            "the draft has {} line(s) but compiles to {}, which is a bug in `import`",
            spoken.len(),
            lines.len()
        ));
    }
    let audio = mono(pcm);
    let rate = u64::from(pcm.sample_rate);
    let mut takes = Takes::load(&project.takes_dir()).map_err(|e| e.to_string())?;
    let mut saved = Vec::new();
    for (i, ((id, text), line)) in lines.iter().zip(&spoken).enumerate() {
        let after_prev = i
            .checked_sub(1)
            .map_or(0, |p| (spoken[p].end_ms + line.start_ms) / 2);
        let before_next = spoken
            .get(i + 1)
            .map_or(u64::MAX, |n| (line.end_ms + n.start_ms) / 2);
        let start = line.start_ms.saturating_sub(LEAD_MS).max(after_prev);
        let end = (line.end_ms + TAIL_MS).min(before_next);
        // From the recording's clock to the voice's.
        let at = |ms: u64| {
            let ms = ms.saturating_add_signed(-offset_ms);
            ((ms * rate / 1000) as usize).min(audio.len())
        };
        let clip = Pcm {
            sample_rate: pcm.sample_rate,
            channels: 1,
            samples: audio[at(start)..at(end)]
                .iter()
                .map(|&s| (s * 32768.0).round().clamp(-32768.0, 32767.0) as i16)
                .collect(),
        };
        takes.save(id, text, &clip).map_err(|e| e.to_string())?;
        saved.push(id.clone());
    }
    Ok(saved)
}

/// The script's heading, from its file name.
fn title_of(script: &Path) -> String {
    let stem = script.file_stem().map_or_else(
        || "Recording".to_string(),
        |s| s.to_string_lossy().replace(['-', '_'], " "),
    );
    let mut chars = stem.chars();
    chars.next().map_or_else(
        || "Recording".to_string(),
        |c| c.to_uppercase().chain(chars).collect(),
    )
}
