//! `teleprompt import <cast> --voice <wav>`: a recorded terminal session
//! and the voice recorded with it become a script, and the takes its lines
//! are spoken from. The deriving is `teleprompt-derive`; this reads the
//! files, hears the voice, and writes the results.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use teleprompt_derive::{derive, read_cast, Draft, Line, Options, Word};
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
    pub cast: &'a Path,
    pub voice: &'a Path,
    pub script: &'a Path,
    pub words: Words<'a>,
    /// How long after the cast started the voice recording did.
    pub offset_ms: i64,
    /// Overwrite the script and its lines' takes.
    pub force: bool,
}

#[derive(Debug, Serialize)]
pub struct ImportReport {
    pub created: PathBuf,
    pub lines: usize,
    pub tapes: usize,
    /// The lines now spoken from the recording, by id.
    pub takes: Vec<String>,
}

impl ImportReport {
    pub fn render(&self) -> String {
        format!(
            "drafted {} from the session: {} line(s), {} tape(s), {} take(s) recorded\n  \
             the prose is what was said, verbatim: edit it, then record the lines you \
             change again with `teleprompt prompt`\n",
            self.created.display(),
            self.lines,
            self.tapes,
            self.takes.len()
        )
    }
}

/// How much of the recording a take keeps around its words.
const LEAD_MS: u64 = 150;
const TAIL_MS: u64 = 250;

pub fn run_import(imp: &Import) -> Result<ImportReport, String> {
    if imp.script.exists() && !imp.force {
        return Err(format!(
            "{} already exists; pass --force to replace it and its lines' takes",
            imp.script.display()
        ));
    }
    let project = Project::for_script(imp.script).map_err(|e| e.to_string())?;
    let cast = std::fs::read_to_string(imp.cast)
        .map_err(|e| format!("cannot read {}: {e}", imp.cast.display()))?;
    let trace = read_cast(&cast).map_err(|e| format!("{}: {e}", imp.cast.display()))?;
    let pcm = read_voice(imp.voice)?;
    let words = hear(&imp.words, &pcm, imp.offset_ms)?;
    if words.is_empty() {
        return Err(format!("nothing was heard in {}", imp.voice.display()));
    }

    let options = Options {
        title: title_of(imp.script),
        ..Options::default()
    };
    let draft = derive(&trace, &words, &options);
    let partial = imp.script.with_extension("md.partial");
    std::fs::write(&partial, draft.markdown(&options))
        .and_then(|()| std::fs::rename(&partial, imp.script))
        .map_err(|e| format!("cannot write {}: {e}", imp.script.display()))?;

    // The script as the compiler reads it, which is what names its lines.
    let (compiled, _) = compile_script(&project, imp.script, "en").map_err(|errors| {
        format!(
            "the drafted {} does not compile, which is a bug in `import`:\n{}",
            imp.script.display(),
            errors.join("\n")
        )
    })?;
    let lines: Vec<(String, String)> = compiled
        .narration
        .iter()
        .map(|n| (n.line_id.clone(), n.text.clone()))
        .collect();
    let takes = save_takes(&project, &draft, &lines, &pcm, imp.offset_ms)?;
    Ok(ImportReport {
        created: imp.script.to_path_buf(),
        lines: lines.len(),
        tapes: draft.beats.iter().map(|b| b.blocks.len()).sum(),
        takes,
    })
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

/// The words in the voice, on the cast's clock.
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
        // From the cast's clock to the recording's.
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
