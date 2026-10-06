//! Drafting a script from the transcript of a conversation: captions
//! (WebVTT, SRT) or text. Each turn becomes a line opening with who says
//! it, `**Ada Lovelace:**`, and the people in it the script's cast.

use std::collections::BTreeMap;
use std::path::Path;

use teleprompt_core::attrs::is_speaker_name;
use teleprompt_listen::SpeakerSpan;
use teleprompt_script::ast::slugify;

use crate::document::{id_for, unique};

/// Words past which a turn read from captions goes on in another line,
/// from the next cue that ends a sentence.
const LONGEST_LINE: usize = 40;

/// The most words a name before a colon may have: `Dr. Ada King:` is a
/// name, `And then I said:` is not.
const LONGEST_NAME: usize = 4;

/// One stretch of a conversation: who says it, where the transcript says,
/// and when in the recording, where it says that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub speaker: Option<String>,
    pub text: String,
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// WebVTT, whose speakers are `<v Name>` spans or `Name:`.
    Vtt,
    /// SubRip, whose speakers are `Name:`.
    Srt,
    /// Paragraphs, each opening with `Name:`, `**Name:**`, or a line of
    /// its own naming the speaker and when, as `Ada Lovelace  0:03`.
    Text,
}

impl Format {
    /// The caption format `path`'s extension names, if any.
    pub fn of(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "vtt" => Some(Self::Vtt),
            "srt" => Some(Self::Srt),
            _ => None,
        }
    }
}

/// The turns of `src`. A stretch that names no speaker is the last one's,
/// as a transcript means it. One that says when it starts but not when it
/// ends, as text transcripts do, ends where the next starts.
pub fn turns(src: &str, format: Format) -> Vec<Turn> {
    let src = src.replace("\r\n", "\n");
    let mut turns = match format {
        Format::Vtt | Format::Srt => captions(&src),
        Format::Text => paragraphs(&src),
    };
    for i in 1..turns.len() {
        if turns[i - 1].end_ms.is_none() {
            turns[i - 1].end_ms = turns[i].start_ms;
        }
    }
    turns
}

/// A caption or transcript time, `01:02:03.450`, `02:03,450` or `0:03`,
/// in milliseconds.
fn time_ms(s: &str) -> Option<u64> {
    let s = s.trim().trim_matches(['(', ')', '[', ']']);
    let (clock, fraction) = match s.split_once(['.', ',']) {
        Some((clock, f)) => (clock, f),
        None => (s, "0"),
    };
    let mut ms: u64 = 0;
    for part in clock.split(':') {
        ms = ms * 60 + part.parse::<u64>().ok()?;
    }
    let fraction: String = fraction.chars().chain("000".chars()).take(3).collect();
    Some(ms * 1000 + fraction.parse::<u64>().ok()?)
}

/// A caption file's cues, joined into lines: a speaker's run of cues is
/// one line until it is long, then more.
fn captions(src: &str) -> Vec<Turn> {
    let mut out: Vec<Turn> = Vec::new();
    for block in src.split("\n\n") {
        let lines: Vec<&str> = block.lines().collect();
        // Cues have a timing line; the header, NOTE and STYLE don't.
        let Some(timing) = lines.iter().position(|l| l.contains("-->")) else {
            continue;
        };
        let payload = lines[timing + 1..].join(" ");
        let (from, to) = lines[timing].split_once("-->").unwrap_or_default();
        let from = time_ms(from);
        let to = to.split_whitespace().next().and_then(time_ms);
        // A cue two people speak in is shared out by how much each says.
        let parts = voiced(&payload);
        let total: usize = parts.iter().map(|(_, t)| t.len()).sum::<usize>().max(1);
        let mut said = 0;
        for (speaker, text) in parts {
            let at = |n: usize| match (from, to) {
                (Some(f), Some(t)) => Some(f + t.saturating_sub(f) * n as u64 / total as u64),
                _ => None,
            };
            let (start_ms, end_ms) = (at(said), at(said + text.len()));
            said += text.len();
            let text = clean(&text);
            let text = text.trim_start_matches("- ").trim();
            if text.is_empty() {
                continue;
            }
            let (speaker, text) = match speaker {
                Some(s) => (Some(s), text.to_string()),
                None => match name_prefix(text) {
                    Some((name, rest)) => (Some(name), rest.to_string()),
                    None => (None, text.to_string()),
                },
            };
            let continues = out
                .last()
                .is_some_and(|last| speaker.is_none() || last.speaker == speaker);
            match out.last_mut() {
                Some(last) if continues && !full(&last.text) => {
                    last.text.push(' ');
                    last.text.push_str(&text);
                    last.end_ms = end_ms.or(last.end_ms);
                }
                last => {
                    let speaker = speaker.or_else(|| last.and_then(|l| l.speaker.clone()));
                    out.push(Turn {
                        speaker,
                        text,
                        start_ms,
                        end_ms,
                    });
                }
            }
        }
    }
    out
}

/// A recording's turns, from the words heard in it and who speaks when:
/// each word is the voice's whose stretch it overlaps most, or the nearest
/// one's; a turn goes on while one voice does, until it is long. A word
/// that ends the sentence the last turn left unfinished is that turn's: a
/// recognizer stamps a word late, so a turn's last word can fall in the
/// next voice's stretch. Voices are
/// named `Speaker 1`, `Speaker 2`, … in the order they first speak, and
/// without any, every word is the narrator's.
pub fn conversation(words: &[(String, u64, u64)], spans: &[SpeakerSpan]) -> Vec<Turn> {
    let mut order: Vec<usize> = Vec::new();
    let mut out: Vec<Turn> = Vec::new();
    // Parallel to `out`: each turn's voice, as the diarizer numbers it.
    let mut voices: Vec<Option<usize>> = Vec::new();
    for (text, start, end) in words {
        let voice = voice_of(*start, *end, spans);
        let speaker = voice.map(|voice| {
            let n = order.iter().position(|v| *v == voice).unwrap_or_else(|| {
                order.push(voice);
                order.len() - 1
            });
            format!("Speaker {}", n + 1)
        });
        let finishes = |last: &Turn| !ends_sentence(&last.text) && ends_sentence(text);
        match out.last_mut() {
            Some(last) if (last.speaker == speaker && !full(&last.text)) || finishes(last) => {
                last.text.push(' ');
                last.text.push_str(text);
                last.end_ms = Some(*end);
            }
            _ => {
                voices.push(voice);
                out.push(Turn {
                    speaker,
                    text: text.clone(),
                    start_ms: Some(*start),
                    end_ms: Some(*end),
                });
            }
        }
    }
    snap_to_voices(&mut out, &voices, spans);
    // A line opens a sentence, whoever's.
    for turn in &mut out {
        let mut chars = turn.text.chars();
        if let Some(first) = chars.next() {
            turn.text = first.to_uppercase().chain(chars).collect();
        }
    }
    out
}

/// Where the speaker changes, each side's edge is where the diarizer heard
/// that voice start or stop rather than where the recognizer stamped the
/// word, which is late: so a take neither clips its first word nor carries
/// the next speaker's.
fn snap_to_voices(turns: &mut [Turn], voices: &[Option<usize>], spans: &[SpeakerSpan]) {
    for i in 0..turns.len() {
        let Some(voice) = voices[i] else { continue };
        let own: Vec<SpeakerSpan> = spans
            .iter()
            .filter(|s| s.speaker == voice)
            .copied()
            .collect();
        let (Some(start), Some(end)) = (turns[i].start_ms, turns[i].end_ms) else {
            continue;
        };
        let changes = |j: Option<usize>| {
            j.and_then(|j| voices.get(j))
                .is_none_or(|v| *v != Some(voice))
        };
        if changes(i.checked_sub(1)) {
            if let Some(span) = nearest(&own, start, start + 1) {
                turns[i].start_ms = Some(span.start_ms.min(start));
            }
        }
        if changes(Some(i + 1)) {
            if let Some(span) = nearest(&own, end.saturating_sub(1), end) {
                turns[i].end_ms = Some(span.end_ms);
            }
        }
    }
}

/// The span overlapping `start..end` most, or the nearest.
fn nearest(spans: &[SpeakerSpan], start: u64, end: u64) -> Option<SpeakerSpan> {
    let overlap = |s: &SpeakerSpan| end.min(s.end_ms).saturating_sub(start.max(s.start_ms));
    let distance = |s: &SpeakerSpan| {
        if end < s.start_ms {
            s.start_ms - end
        } else {
            start.saturating_sub(s.end_ms)
        }
    };
    spans
        .iter()
        .max_by_key(|s| (overlap(s), std::cmp::Reverse(distance(s))))
        .copied()
}

/// The voice speaking over `start..end`: the one overlapping it most, or,
/// for a word in a gap between them, the nearest.
fn voice_of(start: u64, end: u64, spans: &[SpeakerSpan]) -> Option<usize> {
    nearest(spans, start, end).map(|s| s.speaker)
}

/// Whether a line is long enough to end at the end of its sentence.
fn full(text: &str) -> bool {
    text.split_whitespace().count() >= LONGEST_LINE && ends_sentence(text)
}

fn ends_sentence(text: &str) -> bool {
    text.trim_end_matches(['"', '\'', ')', '”', '’'])
        .ends_with(['.', '!', '?', '…'])
}

/// A cue's text split at its `<v Name>` voice spans, each with its
/// speaker; `None` for text outside one.
fn voiced(payload: &str) -> Vec<(Option<String>, String)> {
    let mut out = Vec::new();
    let mut rest = payload;
    while let Some(at) = rest.find("<v") {
        let after = &rest[at + 2..];
        // `<v Name>` or `<v.class Name>`, not `<video>` or the like.
        if !after.starts_with([' ', '.']) {
            out.push((None, rest[..at + 2].to_string()));
            rest = after;
            continue;
        }
        if !rest[..at].trim().is_empty() {
            out.push((None, rest[..at].to_string()));
        }
        let Some(close) = after.find('>') else {
            break;
        };
        let tag = &after[..close];
        let name = tag.split_once(' ').map_or("", |(_, n)| n).trim();
        let body = &after[close + 1..];
        let end = body
            .find("</v>")
            .or_else(|| body.find("<v"))
            .unwrap_or(body.len());
        let name = (!name.is_empty()).then(|| name.to_string());
        out.push((name, body[..end].to_string()));
        rest = body[end..].strip_prefix("</v>").unwrap_or(&body[end..]);
    }
    if !rest.trim().is_empty() {
        out.push((None, rest.to_string()));
    }
    out
}

/// A text transcript's turns, a paragraph each.
fn paragraphs(src: &str) -> Vec<Turn> {
    let mut out: Vec<Turn> = Vec::new();
    for block in src.split("\n\n") {
        let lines: Vec<&str> = block
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        let Some(first) = lines.first() else {
            continue;
        };
        let (speaker, text, start_ms) = match header(first) {
            // `Ada Lovelace  0:03`, then what she said.
            Some((name, at)) if lines.len() > 1 => (Some(name), lines[1..].join(" "), at),
            _ => {
                let joined = lines.join(" ");
                let (text, at) = without_timestamp(&joined);
                let (name, text) = match label(text).or_else(|| name_prefix(text)) {
                    Some((name, rest)) => (Some(name), rest.to_string()),
                    None => (None, text.to_string()),
                };
                (
                    name.clone(),
                    text,
                    at.or_else(|| name.as_deref().and(stamped(&joined))),
                )
            }
        };
        let text = clean(&text);
        if text.is_empty() {
            continue;
        }
        let speaker = speaker.or_else(|| out.last().and_then(|t| t.speaker.clone()));
        out.push(Turn {
            speaker,
            text,
            start_ms,
            end_ms: None,
        });
    }
    out
}

/// A line naming a speaker and when they began, as meeting tools write
/// them: `Ada Lovelace  0:03`, `Ada Lovelace (00:01:02)`.
fn header(line: &str) -> Option<(String, Option<u64>)> {
    let (name, time) = line.trim_end().rsplit_once(char::is_whitespace)?;
    let time = time.trim_matches(['(', ')', '[', ']']);
    (is_timestamp(time) && is_name(name.trim())).then(|| (name.trim().to_string(), time_ms(time)))
}

/// The time in `Name (0:03): …`, between a name and its colon.
fn stamped(text: &str) -> Option<u64> {
    let (before, _) = text.split_once(": ")?;
    let (_, last) = before.rsplit_once(char::is_whitespace)?;
    let last = last.trim_matches(['(', ')', '[', ']']);
    is_timestamp(last).then(|| time_ms(last)).flatten()
}

/// `**Name:** rest` or `**Name**: rest`.
fn label(text: &str) -> Option<(String, &str)> {
    let mark = ["**", "__"].into_iter().find(|m| text.starts_with(m))?;
    let close = text[2..].find(mark)? + 2;
    let inner = text[2..close].trim();
    let after = &text[close + 2..];
    let (name, rest) = match inner.strip_suffix(':') {
        Some(name) => (name, after),
        None => (inner, after.strip_prefix(':')?),
    };
    let name = name.trim();
    (is_name(name) && !rest.trim().is_empty()).then(|| (name.to_string(), rest.trim()))
}

/// `Name: rest`, or `Name (0:03): rest`, where what comes before the colon
/// reads as a name.
fn name_prefix(text: &str) -> Option<(String, &str)> {
    // The colon a space follows: `(01:02)` has one that isn't.
    let at = text
        .match_indices(':')
        .find(|(i, _)| text[i + 1..].starts_with(char::is_whitespace))?
        .0;
    let (before, rest) = (&text[..at], &text[at + 1..]);
    let name = without_timestamp_after(before.trim());
    (is_name(name) && !rest.trim().is_empty()).then(|| (name.to_string(), rest.trim()))
}

/// Words a name may have in lower case: `Ada de Lovelace`.
const PARTICLES: &[&str] = &[
    "de", "da", "del", "der", "van", "von", "la", "le", "of", "bin",
];

/// Whether `s` reads as someone's name: a few words, each capitalized but
/// for particles, and no sentence in it.
fn is_name(s: &str) -> bool {
    let words: Vec<&str> = s.split_whitespace().collect();
    (1..=LONGEST_NAME).contains(&words.len())
        && s.starts_with(|c: char| c.is_uppercase() || c.is_ascii_digit())
        && words.iter().all(|w| {
            w.starts_with(|c: char| c.is_uppercase() || c.is_ascii_digit()) || PARTICLES.contains(w)
        })
        && !s.contains(['!', '?', ',', ';', '"', '/', '*'])
        && !s.contains("http")
}

fn is_timestamp(s: &str) -> bool {
    s.contains(':')
        && s.chars()
            .all(|c| c.is_ascii_digit() || matches!(c, ':' | '.' | ','))
}

/// `text` without a timestamp opening it, `[00:01:02]`, `(0:03)` or
/// `0:03`, and the time it said.
fn without_timestamp(text: &str) -> (&str, Option<u64>) {
    let Some((first, rest)) = text.split_once(char::is_whitespace) else {
        return (text, None);
    };
    let first = first.trim_matches(['(', ')', '[', ']']);
    if is_timestamp(first) {
        (rest.trim_start(), time_ms(first))
    } else {
        (text, None)
    }
}

/// A name without the timestamp after it: `Ada (0:03)` is `Ada`.
fn without_timestamp_after(name: &str) -> &str {
    match name.rsplit_once(char::is_whitespace) {
        Some((before, last)) if is_timestamp(last.trim_matches(['(', ')', '[', ']'])) => {
            before.trim_end()
        }
        _ => name,
    }
}

/// Caption text as it is said: its tags gone, its entities read, its
/// whitespace single.
fn clean(text: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A draft script from a transcript, and who is in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptDraft {
    pub script: String,
    /// The cast, as its keys under `voices`, in the order they first speak.
    pub cast: Vec<String>,
    /// Lines no one is named as saying, which are the narrator's.
    pub unattributed: usize,
}

/// Drafts a script from `turns`: a chapter named `title`, a line per turn
/// opening with its speaker's label, and in the front matter a cast of
/// everyone, each yet to be given a voice.
pub fn draft_transcript(turns: &[Turn], title: &str) -> TranscriptDraft {
    // Keyed by slug, so `Ada` and `ADA` are one person, labelled as first
    // written.
    let mut cast: Vec<(String, String)> = Vec::new();
    let mut seen = BTreeMap::new();
    let mut body = format!("# {title}\n\n");
    let mut unattributed = 0;
    for turn in turns {
        let label = match &turn.speaker {
            Some(name) => {
                let (key, label) = member(name);
                match cast.iter().find(|(k, _)| *k == key) {
                    Some((_, first)) => Some(first.clone()),
                    None => {
                        cast.push((key, label.clone()));
                        Some(label)
                    }
                }
            }
            None => {
                unattributed += 1;
                None
            }
        };
        let id = unique(id_for(&turn.text), &mut seen);
        let id = if id.is_empty() {
            unique("line".into(), &mut seen)
        } else {
            id
        };
        let text = escaped(&turn.text);
        match label {
            Some(label) => body.push_str(&format!("**{label}:** {text} {{#{id}}}\n\n")),
            None => body.push_str(&format!("{text} {{#{id}}}\n\n")),
        }
    }
    let mut script = String::from("---\nteleprompt: 1\n");
    if !cast.is_empty() {
        script.push_str(
            "# Each speaker reads in the narrator's voice until given their own,\n\
             # as in `voice: am_michael`, or `backend:` and `instruct:`.\n\
             voices:\n",
        );
        for (key, _) in &cast {
            script.push_str(&format!("  {key}: {{}}\n"));
        }
    }
    script.push_str("---\n\n");
    script.push_str(&body);
    TranscriptDraft {
        script,
        cast: cast.into_iter().map(|(k, _)| k).collect(),
        unattributed,
    }
}

/// A speaker's key in the cast and the label that names it: `Ada
/// Lovelace` is `ada-lovelace`. A name that makes no key, such as `2`,
/// becomes `Speaker 2`, `speaker-2`.
fn member(name: &str) -> (String, String) {
    let key = slugify(name);
    if is_speaker_name(&key) {
        (key, titled(name))
    } else {
        let label = format!("Speaker {name}");
        (slugify(&label), label)
    }
}

/// A name shouted in capitals, as captions write them, in title case:
/// `HOST` is `Host`, `ADA LOVELACE` is `Ada Lovelace`. Others as written.
fn titled(name: &str) -> String {
    if name.chars().any(char::is_lowercase) {
        return name.to_string();
    }
    name.split(' ')
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|c| {
                    c.to_uppercase()
                        .chain(chars.flat_map(char::to_lowercase))
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect::<Vec<String>>()
        .join(" ")
}

/// `text` kept a paragraph: what would open a heading, quote or list is
/// escaped, and a closing `}` is not read as attributes.
fn escaped(text: &str) -> String {
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    let mut out = if text.starts_with(['#', '>', '-', '+', '*', '=']) {
        format!("\\{text}")
    } else if digits > 0 && text[digits..].starts_with(['.', ')']) {
        format!("{}\\{}", &text[..digits], &text[digits..])
    } else {
        text.to_string()
    };
    if out.ends_with('}') {
        out.push('.');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn said(turns: &[Turn]) -> Vec<(Option<&str>, &str)> {
        turns
            .iter()
            .map(|t| (t.speaker.as_deref(), t.text.as_str()))
            .collect()
    }

    const VTT: &str = "WEBVTT\n\nNOTE made by hand\n\n\
        1\n00:00:00.000 --> 00:00:02.000\n<v Ada Lovelace>The engine weaves\n\n\
        00:00:02.000 --> 00:00:04.000\n<v Ada Lovelace>algebraic patterns.</v>\n\n\
        00:00:04.000 --> 00:00:06.000\n<v.loud Charles Babbage>Just as the loom\n<i>weaves</i> flowers &amp; leaves.\n\n\
        00:00:06.000 --> 00:00:08.000\n<v Ada Lovelace>Quite.</v> <v Charles Babbage>Indeed.</v>\n";

    #[test]
    fn webvtt_voices_are_the_speakers_and_their_cues_join() {
        assert_eq!(
            said(&turns(VTT, Format::Vtt)),
            [
                (
                    Some("Ada Lovelace"),
                    "The engine weaves algebraic patterns."
                ),
                (
                    Some("Charles Babbage"),
                    "Just as the loom weaves flowers & leaves."
                ),
                (Some("Ada Lovelace"), "Quite."),
                (Some("Charles Babbage"), "Indeed."),
            ]
        );
    }

    #[test]
    fn each_turn_is_where_it_is_in_the_recording() {
        let spans = |t: Vec<Turn>| -> Vec<(Option<u64>, Option<u64>)> {
            t.iter().map(|t| (t.start_ms, t.end_ms)).collect()
        };
        // Joined cues run first to last; a shared cue is shared by length.
        assert_eq!(
            spans(turns(VTT, Format::Vtt)),
            [
                (Some(0), Some(4000)),
                (Some(4000), Some(6000)),
                (Some(6000), Some(6923)),
                (Some(6923), Some(8000)),
            ]
        );
        let text = "Ada  0:03\nHello.\n\n[00:01:02.5] Charles: Hi.\n\nBob (1:10): Bye.\n";
        assert_eq!(
            spans(turns(text, Format::Text)),
            [
                (Some(3000), Some(62_500)),
                (Some(62_500), Some(70_000)),
                (Some(70_000), None),
            ]
        );
    }

    #[test]
    fn a_recordings_words_go_to_whoever_speaks_over_them() {
        let w = |t: &str, s: u64, e: u64| (t.to_string(), s, e);
        let words = [
            w("Welcome", 0, 400),
            w("back.", 400, 900),
            // In the gap between voices: the nearer one's.
            w("Thanks!", 1_150, 1_600),
            w("Glad", 1_700, 2_000),
            w("to", 2_000, 2_200),
            w("be", 2_200, 2_400),
            w("here.", 2_400, 2_900),
            w("Great.", 3_100, 3_500),
        ];
        let span = |s, e, speaker| SpeakerSpan {
            start_ms: s,
            end_ms: e,
            speaker,
        };
        // Diarization numbers voices in no order; the drafts count them as
        // they first speak.
        let spans = [
            span(0, 1_000, 7),
            span(1_200, 3_000, 2),
            span(3_000, 3_600, 7),
        ];
        // Its last word stamped in the next voice's stretch, the sentence
        // is still Speaker 2's.
        let mut late = words.to_vec();
        late[6].1 = 3_050;
        late[6].2 = 3_090;
        assert_eq!(
            conversation(&late, &spans)[1].text,
            "Thanks! Glad to be here."
        );
        let t = conversation(&words, &spans);
        type Said<'a> = (Option<&'a str>, &'a str, Option<u64>, Option<u64>);
        let said: Vec<Said> = t
            .iter()
            .map(|t| (t.speaker.as_deref(), t.text.as_str(), t.start_ms, t.end_ms))
            .collect();
        assert_eq!(
            said,
            [
                // Edges where the voices start and stop, not where the
                // words were stamped.
                (Some("Speaker 1"), "Welcome back.", Some(0), Some(1_000)),
                (
                    Some("Speaker 2"),
                    "Thanks! Glad to be here.",
                    Some(1_150),
                    Some(3_000)
                ),
                (Some("Speaker 1"), "Great.", Some(3_000), Some(3_600)),
            ]
        );
        // No voices told apart: one narrator, one line.
        let alone = conversation(&words[..2], &[]);
        assert_eq!(alone.len(), 1);
        assert_eq!(alone[0].speaker, None);
    }

    #[test]
    fn srt_names_a_speaker_when_the_turn_changes() {
        let srt = "1\r\n00:00:01,000 --> 00:00:02,000\r\nHOST: Welcome back.\r\n\r\n\
                   2\r\n00:00:02,000 --> 00:00:03,000\r\nToday, a guest.\r\n\r\n\
                   3\r\n00:00:03,000 --> 00:00:04,000\r\nGUEST: Thanks for having me.\r\n";
        assert_eq!(
            said(&turns(srt, Format::Srt)),
            [
                (Some("HOST"), "Welcome back. Today, a guest."),
                (Some("GUEST"), "Thanks for having me."),
            ]
        );
    }

    #[test]
    fn a_long_turn_goes_on_in_another_line_after_a_sentence() {
        let sentence = "one two three four five six seven eight nine ten.";
        let cues: String = (0..6)
            .map(|i| format!("00:00:0{i}.000 --> 00:00:0{i}.900\n<v Ada>{sentence}\n\n"))
            .collect();
        let t = turns(&format!("WEBVTT\n\n{cues}"), Format::Vtt);
        assert_eq!(t.len(), 2, "{t:?}");
        assert_eq!(t[0].text.split_whitespace().count(), 40);
        assert!(t.iter().all(|t| t.speaker.as_deref() == Some("Ada")));
    }

    #[test]
    fn text_transcripts_in_the_usual_shapes() {
        let text = "Ada Lovelace  0:03\nThe engine weaves\nalgebraic patterns.\n\n\
                    [00:00:09] Charles Babbage: As the loom weaves flowers.\n\n\
                    **Ada:** Quite so.\n\n\
                    And more besides.\n\n\
                    Mr. Menabrea (01:02): I wrote it first.\n\n\
                    And then I said: no.\n";
        assert_eq!(
            said(&turns(text, Format::Text)),
            [
                (
                    Some("Ada Lovelace"),
                    "The engine weaves algebraic patterns."
                ),
                (Some("Charles Babbage"), "As the loom weaves flowers."),
                (Some("Ada"), "Quite so."),
                // A paragraph naming no one goes on with who spoke last.
                (Some("Ada"), "And more besides."),
                (Some("Mr. Menabrea"), "I wrote it first."),
                (Some("Mr. Menabrea"), "And then I said: no."),
            ]
        );
    }

    #[test]
    fn the_draft_labels_each_line_and_casts_everyone() {
        let t = [
            Turn {
                speaker: Some("Ada Lovelace".into()),
                text: "The engine weaves.".into(),
                start_ms: None,
                end_ms: None,
            },
            Turn {
                speaker: Some("ADA LOVELACE".into()),
                text: "# of patterns: many.".into(),
                start_ms: None,
                end_ms: None,
            },
            Turn {
                speaker: Some("2".into()),
                text: "1. First {this}".into(),
                start_ms: None,
                end_ms: None,
            },
            Turn {
                speaker: None,
                text: "The engine weaves.".into(),
                start_ms: None,
                end_ms: None,
            },
        ];
        let d = draft_transcript(&t, "Interview");
        assert_eq!(d.cast, ["ada-lovelace", "speaker-2"]);
        assert_eq!(member("HOST"), ("host".into(), "Host".into()));
        assert_eq!(member("McCoy"), ("mccoy".into(), "McCoy".into()));
        assert_eq!(d.unattributed, 1);
        assert!(d
            .script
            .contains("voices:\n  ada-lovelace: {}\n  speaker-2: {}\n"));
        let body = &d.script[d.script.find("# Interview").unwrap()..];
        assert_eq!(
            body,
            "# Interview\n\n\
             **Ada Lovelace:** The engine weaves. {#the-engine-weaves}\n\n\
             **Ada Lovelace:** \\# of patterns: many. {#of-patterns}\n\n\
             **Speaker 2:** 1\\. First {this}. {#1-first-this}\n\n\
             The engine weaves. {#the-engine-weaves-2}\n\n"
        );
    }
}
