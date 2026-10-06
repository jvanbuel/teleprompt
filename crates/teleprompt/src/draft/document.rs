//! `teleprompt import <doc>`: a document, deck or conversation drafted into
//! a script. The file handling around `teleprompt_derive::document` and
//! `::transcript`, which do the drafting; a recorded session is `import`'s
//! other half, `crate::draft::import`.

use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};

use teleprompt_core::ast::slugify;
use teleprompt_core::LineId;
use teleprompt_voice::Pcm;

use crate::draft::import::{cut_takes, read_voice};
use crate::project::Project;
use serde::Serialize;
use teleprompt_derive::document::{draft, draft_slidev};
use teleprompt_derive::transcript::{conversation, draft_transcript, turns, Format, Turn};

/// The stable, typed shape of `import`'s output in both formats.
#[derive(Debug, Serialize)]
pub struct DocumentReport {
    pub source: PathBuf,
    pub created: PathBuf,
    /// Action blocks written as `review=pending`, which `check` will warn
    /// about until a human has read them.
    pub unreviewed: usize,
    /// Slides of a Slidev deck with no speaker notes, so nothing to say
    /// over them, and so not in the draft.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub silent_slides: Vec<u32>,
    /// Lines given their stretch of the conversation's recording as
    /// their take, with `--audio`.
    #[serde(skip_serializing_if = "is_zero")]
    pub takes: usize,
    /// A transcript's speakers, as their keys in the draft's cast.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cast: Vec<String>,
    /// What the draft could not follow, one sentence each.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl DocumentReport {
    pub fn render(&self) -> String {
        let mut s = format!(
            "drafted {} from {}\n",
            self.created.display(),
            self.source.display()
        );
        if self.unreviewed > 0 {
            s.push_str(&format!(
                "  {} tape(s) marked `review=pending` — read them before dubbing\n",
                self.unreviewed
            ));
        }
        if !self.silent_slides.is_empty() {
            let list: Vec<String> = self.silent_slides.iter().map(u32::to_string).collect();
            s.push_str(&format!(
                "  slide(s) {} have no speaker notes, so nothing is said over them and they \
                 are not in the draft\n",
                list.join(", ")
            ));
        }
        if self.takes > 0 {
            s.push_str(&format!(
                "  {} line(s) speak from the recording; reword one and it is spoken by \
                 its speaker's voice instead\n",
                self.takes
            ));
        }
        if !self.cast.is_empty() {
            s.push_str(&format!(
                "  cast: {}; each reads in the narrator's voice until you give them one \
                 under `voices` in its front matter\n",
                self.cast.join(", ")
            ));
        }
        for w in &self.warnings {
            s.push_str(&format!("  warning: {w}\n"));
        }
        s
    }
}

/// The chapter name for prose before the document's first heading, taken
/// from the file name.
fn title_of(doc: &Path) -> String {
    let stem = doc
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Introduction".to_string());
    let spaced = stem.replace(['-', '_'], " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => "Introduction".to_string(),
    }
}

fn default_out(doc: &Path) -> PathBuf {
    doc.with_extension("teleprompt.md")
}

/// What `import` reads a document as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    Document,
    Slidev,
    /// A conversation: captions, by their extension, or a text transcript.
    Transcript,
    /// A conversation's recording, by its extension: transcribed, and its
    /// voices told apart.
    Recording,
}

/// Extensions read as a recording rather than a document.
const RECORDINGS: &[&str] = &[
    "wav", "m4a", "mp3", "flac", "ogg", "opus", "aac", "mp4", "mov", "mkv", "webm",
];

fn is_recording(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| RECORDINGS.contains(&e.to_ascii_lowercase().as_str()))
}

/// A conversation's recording, for `import` to give each line its stretch
/// of as its take, but for the lines of the speakers in `revoice`: `audio`
/// beside a transcript, or the document itself when it is a recording.
/// `speakers` is how many voices a recording has, where known.
#[derive(Debug, Clone, Copy, Default)]
pub struct Audio<'a> {
    pub path: Option<&'a Path>,
    pub revoice: &'a [String],
    pub speakers: Option<usize>,
}

pub fn run_document(
    doc: &Path,
    out: Option<PathBuf>,
    reading: Reading,
    audio: Audio,
) -> std::io::Result<DocumentReport> {
    let created = out.unwrap_or_else(|| default_out(doc));
    if created.exists() {
        return Err(Error::new(
            ErrorKind::AlreadyExists,
            format!(
                "{} already exists; drafting over it would discard whatever \
                 you have written there",
                created.display()
            ),
        ));
    }

    let reading = match (reading, Format::of(doc)) {
        (Reading::Document, Some(_)) => Reading::Transcript,
        (Reading::Document, None) if is_recording(doc) => Reading::Recording,
        (r, _) => r,
    };
    let invalid = |why: &str| Err(Error::new(ErrorKind::InvalidInput, why.to_string()));
    match (reading, audio.path) {
        (Reading::Recording, Some(_)) => {
            return invalid("the document is the recording, so --audio has nothing to add")
        }
        (Reading::Document | Reading::Slidev, Some(_)) => {
            return invalid("--audio is the recording of a conversation, for a transcript of it")
        }
        (Reading::Document | Reading::Slidev, None) if !audio.revoice.is_empty() => {
            return invalid("--revoice names someone in a conversation's recording")
        }
        _ => {}
    }
    // A recording is read before the draft is written, so a wrong one
    // leaves nothing behind.
    let (drafted, pcm) = if reading == Reading::Recording {
        let pcm = recording(doc, &[])?;
        (heard_draft(doc, &pcm, audio.speakers)?, Some(pcm))
    } else {
        let source = std::fs::read_to_string(doc)
            .map_err(|e| Error::new(e.kind(), format!("cannot read {}: {e}", doc.display())))?;
        (drafted(doc, &source, reading)?, None)
    };
    if let Some(parent) = created.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let conversation = drafted.conversation.unwrap_or_default();
    let voice = match (pcm, audio.path) {
        (Some(pcm), _) => Some((doc, pcm, revoiced(doc, &audio, &conversation)?)),
        (None, Some(path)) => Some((
            path,
            recording(path, &conversation.spans)?,
            revoiced(doc, &audio, &conversation)?,
        )),
        (None, None) => None,
    };
    std::fs::write(&created, &drafted.script)?;
    let takes = match voice {
        Some((path, pcm, revoice)) => {
            let spans: Vec<(u64, u64)> = conversation
                .spans
                .iter()
                .map(|&(s, e)| (s.unwrap_or(0), e.unwrap_or(u64::MAX)))
                .collect();
            let keep = |i: usize| {
                conversation.speakers[i]
                    .as_ref()
                    .is_none_or(|speaker| !revoice.contains(speaker))
            };
            speak_from(&created, &spans, &pcm, keep)
                .map_err(|e| Error::other(format!("{}: {e}", path.display())))?
        }
        None => 0,
    };
    let (script, silent_slides, warnings, cast) = (
        drafted.script,
        drafted.silent,
        drafted.warnings,
        conversation.cast,
    );

    Ok(DocumentReport {
        source: doc.to_path_buf(),
        created,
        unreviewed: script.matches("review=pending").count(),
        silent_slides,
        takes,
        cast,
        warnings,
    })
}

/// A draft, and for a transcript, who and when.
struct Drafted {
    script: String,
    silent: Vec<u32>,
    warnings: Vec<String>,
    conversation: Option<Conversation>,
}

/// A transcript's cast, and each line's speaker, by slug, and stretch.
#[derive(Default)]
struct Conversation {
    cast: Vec<String>,
    speakers: Vec<Option<String>>,
    spans: Vec<(Option<u64>, Option<u64>)>,
}

fn drafted(doc: &Path, source: &str, reading: Reading) -> std::io::Result<Drafted> {
    let plain = |script| Drafted {
        script,
        silent: Vec::new(),
        warnings: Vec::new(),
        conversation: None,
    };
    match reading {
        // The deck path goes into the draft as given, relative to where
        // teleprompt runs; its imports are read relative to the deck itself.
        Reading::Slidev => {
            let dir = doc.parent().unwrap_or(Path::new(""));
            let read = |path: &str| std::fs::read_to_string(dir.join(path)).ok();
            let d = draft_slidev(source, &doc.display().to_string(), &read);
            Ok(Drafted {
                silent: d.silent,
                warnings: d.warnings,
                ..plain(d.script)
            })
        }
        Reading::Transcript | Reading::Recording => {
            let turns = turns(source, Format::of(doc).unwrap_or(Format::Text));
            let mut drafted = conversation_draft(doc, &turns)?;
            let d = drafted.conversation.as_ref();
            if d.is_some_and(|c| c.cast.is_empty()) {
                drafted.warnings.push(
                    "no one is named as speaking, so every line is the narrator's; \
                     a transcript names them as `Name:` or `<v Name>`"
                        .to_string(),
                );
            }
            Ok(drafted)
        }
        Reading::Document => Ok(plain(draft(source, &title_of(doc)))),
    }
}

/// A draft of the conversation in `pcm`, heard with the models `setup`
/// installed.
fn heard_draft(doc: &Path, pcm: &Pcm, speakers: Option<usize>) -> std::io::Result<Drafted> {
    let heard = crate::draft::listening::hear(pcm, speakers).map_err(Error::other)?;
    let turns = conversation(&heard.words, heard.voices.as_deref().unwrap_or_default());
    let mut drafted = conversation_draft(doc, &turns)?;
    if heard.voices.is_none() {
        drafted.warnings.push(
            "the speaker models are not installed, so the voices were not told apart and \
             every line is the narrator's: `teleprompt setup speaker-model` installs them"
                .to_string(),
        );
    }
    Ok(drafted)
}

/// A conversation's draft: a line per turn, its speaker's label opening it.
fn conversation_draft(doc: &Path, turns: &[Turn]) -> std::io::Result<Drafted> {
    if turns.is_empty() {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("{} has nothing said in it to draft from", doc.display()),
        ));
    }
    let d = draft_transcript(turns, &title_of(doc));
    let mut warnings = Vec::new();
    if !d.cast.is_empty() && d.unattributed > 0 {
        warnings.push(format!(
            "{} line(s) before anyone is named are the narrator's",
            d.unattributed
        ));
    }
    Ok(Drafted {
        script: d.script,
        silent: Vec::new(),
        warnings,
        conversation: Some(Conversation {
            cast: d.cast,
            speakers: turns
                .iter()
                .map(|t| t.speaker.as_deref().map(slugify))
                .collect(),
            spans: turns.iter().map(|t| (t.start_ms, t.end_ms)).collect(),
        }),
    })
}

/// The speakers to leave to the cast, by slug.
fn revoiced(
    doc: &Path,
    audio: &Audio,
    conversation: &Conversation,
) -> std::io::Result<Vec<String>> {
    let revoice: Vec<String> = audio.revoice.iter().map(|s| slugify(s)).collect();
    if let Some(nobody) = revoice
        .iter()
        .find(|r| !conversation.speakers.contains(&Some((*r).clone())))
    {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "--revoice {nobody}: no one of that name speaks in {}",
                doc.display()
            ),
        ));
    }
    Ok(revoice)
}

/// The conversation's recording, as one channel, for turns that all say
/// when they start. A WAV is read as it is; anything else ffmpeg decodes.
fn recording(path: &Path, spans: &[(Option<u64>, Option<u64>)]) -> std::io::Result<Pcm> {
    if let Some(n) = spans.iter().position(|(start, _)| start.is_none()) {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!(
                "turn {} of the transcript says nothing of when it was said, so its \
                 audio cannot be found; --audio needs captions, or a time on every turn",
                n + 1
            ),
        ));
    }
    let wav = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("wav"));
    if wav {
        return read_voice(path).map_err(Error::other);
    }
    let decoded = std::env::temp_dir().join(format!("teleprompt-from-{}.wav", std::process::id()));
    let status = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-i"])
        .arg(path)
        .args(["-ac", "1", "-f", "wav"])
        .arg(&decoded)
        .status()
        .map_err(|e| {
            Error::new(
                e.kind(),
                format!(
                    "{} is not a WAV, and ffmpeg, which would read it, did not run: {e}",
                    path.display()
                ),
            )
        })?;
    let pcm = if status.success() {
        read_voice(&decoded).map_err(Error::other)
    } else {
        Err(Error::other(format!(
            "ffmpeg could not read {}",
            path.display()
        )))
    };
    let _ = std::fs::remove_file(&decoded);
    pcm
}

/// Gives each of `script`'s lines `keep` keeps its stretch of `pcm` as its take, so it
/// is spoken in its speaker's own voice; how many.
fn speak_from(
    script: &Path,
    spans: &[(u64, u64)],
    pcm: &Pcm,
    keep: impl Fn(usize) -> bool,
) -> Result<usize, String> {
    let project = Project::for_script(script).map_err(|_| {
        format!(
            "{} is not in a teleprompt project, which keeps takes; draft it into one's \
             scripts/ with --out",
            script.display()
        )
    })?;
    let compiled = project
        .script(script, project.source_locale())
        .compile()
        .map_err(|errors| format!("the draft does not compile:\n{}", errors.join("\n")))?
        .output;
    let lines: Vec<(LineId, String)> = compiled
        .narration
        .iter()
        .map(|n| (n.line_id.clone(), n.text.clone()))
        .collect();
    if lines.len() != spans.len() {
        return Err(format!(
            "the draft has {} line(s) for {} turn(s), which is a bug in `import`",
            lines.len(),
            spans.len()
        ));
    }
    Ok(cut_takes(&project, spans, &lines, pcm, 0, keep)?.len())
}
