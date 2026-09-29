//! The prompter: follows a reader through a script by their voice, says
//! which shots to play as they reach them, and records what they read as
//! takes. It knows nothing of how it is shown; `teleprompt prompt` serves
//! it over HTTP, and any other front end drives the same [`Session`].

use std::path::PathBuf;

use teleprompt_compile::CompileOutput;
use teleprompt_core::Hash;
use teleprompt_listen::{Cues, Follower, Recognizer, TakeLog};
use teleprompt_voice::takes::Takes;
use teleprompt_voice::{Pcm, Resampler};

pub use teleprompt_listen::Position;

/// The rate the recognizer listens at. Microphone audio at any other rate
/// is converted for it; a take keeps the rate it was recorded at.
pub const LISTEN_RATE: u32 = 16_000;

/// A shot, and the point in the script at which the reader's voice starts
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShotCue {
    pub shot: String,
    /// Names the shot's clip among the captured ones.
    pub capture_key: Hash,
    pub at: Position,
}

/// Where each shot starts, as the video would start it, but counted in
/// words said rather than milliseconds: an action with no line of its own
/// follows the lines before it, one that starts after its line ends waits
/// for the line to be said, and one that overlaps its line starts on the
/// word the timeline puts it at.
pub fn shot_cues(compiled: &CompileOutput) -> Vec<ShotCue> {
    let mut cues = Vec::new();
    // How many lines the timeline has begun.
    let mut line = 0;
    for entry in &compiled.timeline.entries {
        if let Some(action) = &entry.action {
            let at = match (&entry.narration, compiled.narration.get(line)) {
                (Some(n), _) if action.start_ms >= n.start_ms + n.duration_ms => Position {
                    line: line + 1,
                    word: 0,
                },
                (Some(n), Some(detail)) => Position {
                    line,
                    word: word_at(
                        &detail.text,
                        action.start_ms.saturating_sub(n.start_ms),
                        n.duration_ms,
                    ) + 1,
                },
                _ => Position { line, word: 0 },
            };
            cues.push(ShotCue {
                shot: action.shot.clone(),
                capture_key: action.capture_key,
                at,
            });
        }
        if entry.narration.is_some() {
            line += 1;
        }
    }
    cues
}

/// The word being said `offset_ms` into a line `duration_ms` long, spread
/// over its characters as a cue without word timings is
/// (`docs/design.md#cues`), so that inverting it lands on the cue's word.
fn word_at(text: &str, offset_ms: u64, duration_ms: u64) -> usize {
    if duration_ms == 0 {
        return 0;
    }
    let chars = text.chars().count() as f64;
    let at = (offset_ms as f64 * chars / duration_ms as f64).round() as usize;
    // Words are counted as the aligner counts them: split at whitespace.
    let mut after_space = true;
    let begun = text
        .chars()
        .enumerate()
        .filter(|&(_, c)| {
            let starts = after_space && !c.is_whitespace();
            after_space = c.is_whitespace();
            starts
        })
        .take_while(|&(i, _)| i <= at)
        .count();
    begun.saturating_sub(1)
}

/// What the prompter shows and plays.
#[derive(Debug, Clone)]
pub struct Prompt {
    /// The script's name, as its reader knows it: its file name.
    pub name: String,
    /// The narration, a line per paragraph.
    pub lines: Vec<String>,
    /// Each line's id, which names its take.
    pub ids: Vec<String>,
    /// Every shot, in script order.
    pub shots: Vec<ShotCue>,
    /// Where captured clips are, named by capture key.
    pub clips: PathBuf,
    /// Where takes are recorded to.
    pub takes: PathBuf,
}

/// Where the reader is, and the shots that just reached their cue, in
/// script order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reached {
    pub at: Position,
    pub play: Vec<String>,
}

/// The script as a front end shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    pub name: String,
    pub lines: Vec<ScriptLine>,
    pub shots: Vec<ScriptShot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptLine {
    pub id: String,
    pub text: String,
    /// Whether the line has a take read from it as it now reads.
    pub recorded: bool,
    /// Whether it has a take of other words: reworded since, to be
    /// recorded again.
    pub stale: bool,
    /// The line as its take was heard to say it, where that is other
    /// words: to keep instead of reading it again.
    pub said: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptShot {
    pub shot: String,
    pub capture_key: Hash,
    pub at: Position,
    /// The captured clip, if the shot was captured.
    pub clip: Option<PathBuf>,
}

/// A prompter following one reader: a take at a time, from
/// [`start`](Self::start) to [`stop`](Self::stop).
pub struct Session<R> {
    prompt: Prompt,
    follower: Follower<R>,
    cues: Cues,
    takes: Takes,
    at: Position,
    /// The microphone's rate, and its conversion to [`LISTEN_RATE`].
    resampler: Option<(u32, Resampler)>,
    take: Option<Take>,
}

struct Take {
    /// The line it started on.
    from: usize,
    rate: u32,
    audio: Vec<f32>,
    log: TakeLog,
}

impl<R: Recognizer> Session<R> {
    /// Fails only if the takes directory cannot be read.
    pub fn new(prompt: Prompt, recognizer: R) -> std::io::Result<Self> {
        let texts: Vec<&str> = prompt.lines.iter().map(String::as_str).collect();
        let follower = Follower::new(recognizer, &texts);
        let cues = Cues::new(prompt.shots.iter().map(|s| s.at).collect());
        Ok(Self {
            takes: Takes::load(&prompt.takes)?,
            prompt,
            follower,
            cues,
            at: Position { line: 0, word: 0 },
            resampler: None,
            take: None,
        })
    }

    /// A new take from line `from`; a take not stopped is dropped. The
    /// shots before `from` do not play.
    pub fn start(&mut self, from: usize) -> Reached {
        self.at = Position {
            line: from,
            word: 0,
        };
        self.follower.restart_at(from);
        self.cues.restart_at(self.at);
        self.resampler = None;
        let mut log = TakeLog::new(from);
        log.heard(self.at, 0);
        self.take = Some(Take {
            from,
            rate: LISTEN_RATE,
            audio: Vec::new(),
            log,
        });
        self.reached()
    }

    /// Microphone samples at `rate`, mono: kept for the take, and heard.
    pub fn listen(&mut self, samples: &[f32], rate: u32) -> Reached {
        if let Some(take) = &mut self.take {
            if take.audio.is_empty() {
                take.rate = rate;
            }
            take.audio.extend_from_slice(samples);
        }
        let heard = if rate == LISTEN_RATE {
            samples.to_vec()
        } else {
            if self.resampler.as_ref().is_none_or(|(r, _)| *r != rate) {
                self.resampler = Some((rate, Resampler::new(rate, LISTEN_RATE)));
            }
            let (_, resampler) = self.resampler.as_mut().expect("just set");
            resampler.push(samples)
        };
        if let Some(now) = self.follower.listen(&heard) {
            self.at = now;
            if let Some(take) = &mut self.take {
                take.log.heard(now, take.audio.len());
            }
        }
        self.reached()
    }

    /// Ends the take, keeping each line read in full as that line's take,
    /// with what it is heard to say on its own; the ids of the lines kept.
    pub fn stop(&mut self) -> std::io::Result<Vec<String>> {
        let mut saved = Vec::new();
        let Some(take) = self.take.take() else {
            return Ok(saved);
        };
        let lines = take.log.lines(&take.audio, take.rate);
        let heard = match lines.iter().map(|(line, _)| *line).max() {
            Some(last) => self.heard(&take, last),
            None => Vec::new(),
        };
        for (line, span) in lines {
            let (Some(id), Some(text)) = (self.prompt.ids.get(line), self.prompt.lines.get(line))
            else {
                continue;
            };
            let pcm = Pcm {
                sample_rate: take.rate,
                channels: 1,
                samples: take.audio[span]
                    .iter()
                    .map(|&s| (s * 32768.0).round().clamp(-32768.0, 32767.0) as i16)
                    .collect(),
            };
            match heard.get(line - take.from).filter(|h| !h.is_empty()) {
                Some(heard) => self.takes.save_heard(id, text, heard, &pcm)?,
                None => self.takes.save(id, text, &pcm)?,
            }
            saved.push(id.clone());
        }
        Ok(saved)
    }

    /// What `take` says, heard again whole, so no word is heard cut, and
    /// split among the lines from the one it started on to the one after
    /// `last`, which gets what was begun of it; not the whole script after.
    fn heard(&mut self, take: &Take, last: usize) -> Vec<String> {
        let transcript = if take.rate == LISTEN_RATE {
            self.follower.transcribe(&take.audio)
        } else {
            let at_rate = Resampler::new(take.rate, LISTEN_RATE).push(&take.audio);
            self.follower.transcribe(&at_rate)
        };
        let all = self.prompt.lines.len();
        let lines: Vec<&str> = self.prompt.lines[take.from.min(all)..(last + 2).min(all)]
            .iter()
            .map(String::as_str)
            .collect();
        teleprompt_core::said::per_line(&lines, &transcript)
    }

    /// The script as it was edited: a shot moved or stretched, a line
    /// reworded. Not during a take, which is reading the script it began
    /// with: false then, and nothing changes.
    pub fn replace(&mut self, prompt: Prompt) -> bool {
        if self.take.is_some() {
            return false;
        }
        let texts: Vec<&str> = prompt.lines.iter().map(String::as_str).collect();
        self.follower.set_lines(&texts);
        self.cues = Cues::new(prompt.shots.iter().map(|s| s.at).collect());
        let last = prompt.lines.len().saturating_sub(1);
        self.at = Position {
            line: self.at.line.min(last),
            word: 0,
        };
        self.cues.restart_at(self.at);
        // Its takes as they are now, too: which lines are recorded, and of
        // what words.
        if let Ok(takes) = Takes::load(&prompt.takes) {
            self.takes = takes;
        }
        self.prompt = prompt;
        true
    }

    pub fn script(&self) -> Script {
        let lines = self
            .prompt
            .ids
            .iter()
            .zip(&self.prompt.lines)
            .map(|(id, text)| ScriptLine {
                id: id.clone(),
                text: text.clone(),
                recorded: self.takes.current(id, text).is_some(),
                stale: self.takes.stale(id, text),
                said: self.takes.said(id, text),
            })
            .collect();
        let shots = self
            .prompt
            .shots
            .iter()
            .map(|s| ScriptShot {
                shot: s.shot.clone(),
                capture_key: s.capture_key,
                at: s.at,
                clip: self.clip(&s.capture_key.to_string()),
            })
            .collect();
        Script {
            name: self.prompt.name.clone(),
            lines,
            shots,
        }
    }

    /// The captured clip of a shot in this script, by its capture key;
    /// nothing else in the clip cache.
    pub fn clip(&self, capture_key: &str) -> Option<PathBuf> {
        self.prompt
            .shots
            .iter()
            .find(|s| s.capture_key.to_string() == capture_key)
            .map(|s| self.prompt.clips.join(format!("{}.mp4", s.capture_key)))
            .filter(|path| path.is_file())
    }

    fn reached(&mut self) -> Reached {
        let play = self.prompt.shots[self.cues.reach(self.at)]
            .iter()
            .map(|s| s.shot.clone())
            .collect();
        Reached { at: self.at, play }
    }
}
