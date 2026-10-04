//! What `dub` publishes, from each line's audio: every line's WAV in one
//! format, a `fit-line` line at the tempo the timeline gives it, and the
//! manifest, each line's `audio_hash` the hash of its bytes. `dub` writes
//! it; the prompter plays it. Neither synthesizes here: the caller brings
//! the audio, from the voice cache or a backend.

use teleprompt_core::{Hash, LineId};
use teleprompt_manifest::{AudioInfo, NarrationManifest};
use teleprompt_schedule::Timeline;
use teleprompt_voice::takes::Takes;
use teleprompt_voice::{wav, Pcm};

use crate::{CompileOutput, NarrationDetail};

/// The `AudioInfo` sample rate for a locale with no lines. Any other
/// manifest takes its rate from the audio produced.
pub const NO_AUDIO_SAMPLE_RATE: u32 = 48_000;

/// One line's audio, as a WAV, and its shape.
#[derive(Debug, Clone)]
pub struct LineAudio {
    pub line_id: LineId,
    pub wav: Vec<u8>,
    pub duration_ms: u64,
    pub sample_rate: u32,
    pub channels: u16,
}

impl LineAudio {
    fn of(line_id: LineId, pcm: &Pcm) -> Self {
        Self {
            line_id,
            wav: wav::encode(pcm),
            duration_ms: pcm.duration_ms(),
            sample_rate: pcm.sample_rate,
            channels: pcm.channels,
        }
    }
}

/// The manifest and the audio it names, in document order.
#[derive(Debug, Clone)]
pub struct Published {
    pub manifest: NarrationManifest,
    pub lines: Vec<(LineId, Vec<u8>)>,
}

/// Every line's audio in document order: `synthesized` for the lines
/// without a take, in order, and each recorded line's take, converted to
/// the synthesized lines' rate and channels, since the manifest publishes
/// one.
pub fn with_takes(
    narration: &[NarrationDetail],
    synthesized: Vec<LineAudio>,
    takes: &Takes,
) -> Result<Vec<LineAudio>, String> {
    let mut format = synthesized.first().map(|a| (a.sample_rate, a.channels));
    let mut synthesized = synthesized.into_iter();
    let mut out = Vec::with_capacity(narration.len());
    for detail in narration {
        if detail.take.is_none() {
            out.extend(synthesized.next());
            continue;
        }
        let failed = |e: &dyn std::fmt::Display| format!("line `{}`: {e}", detail.line_id);
        let bytes = takes.read(&detail.line_id).map_err(|e| failed(&e))?;
        let recorded = wav::decode(&bytes).map_err(|e| failed(&e))?;
        let (rate, channels) = *format.get_or_insert((recorded.sample_rate, recorded.channels));
        let pcm = with_channels(recorded.resampled(rate), channels).map_err(|e| failed(&e))?;
        out.push(LineAudio::of(detail.line_id.clone(), &pcm));
    }
    Ok(out)
}

/// The manifest `compiled` publishes with `audio`, every line's in
/// document order: in the first line's format, fitted, and checked against
/// the lengths the manifest states before anything is written, since an
/// output that disagrees with its own manifest is worse than none.
pub fn publish(compiled: &CompileOutput, audio: Vec<LineAudio>) -> Result<Published, String> {
    let audio = in_one_format(audio)?;
    let format = audio.first().map(|a| (a.sample_rate, a.channels));
    let mut lines = Vec::with_capacity(audio.len());
    for a in audio {
        let (bytes, ms) = fitted(&compiled.timeline, &a)?;
        if let Some(published_ms) = published_duration_ms(&compiled.timeline, &a.line_id) {
            if let Some(m) = length_mismatch(&a.line_id, ms, published_ms) {
                return Err(m);
            }
        }
        lines.push((a.line_id, bytes));
    }
    let (sample_rate, channels) = format.unwrap_or((NO_AUDIO_SAMPLE_RATE, 1));
    let mut manifest = crate::manifest::build(
        &compiled.timeline,
        &compiled.chapters,
        &compiled.narration,
        AudioInfo {
            format: "wav".to_string(),
            sample_rate,
            channels,
        },
    );
    // `build` seeds `audio_hash` with the cache key's; the manifest
    // publishes the bytes' (docs/design.md#manifest).
    for (line_id, bytes) in &lines {
        if let Some(entry) = manifest.lines.iter_mut().find(|e| e.id == *line_id) {
            entry.audio_hash = Hash::of(bytes);
        }
    }
    Ok(Published { manifest, lines })
}

/// Every line in the first line's rate and channels: a cast's backends may
/// each speak at their own.
fn in_one_format(audio: Vec<LineAudio>) -> Result<Vec<LineAudio>, String> {
    let Some((rate, channels)) = audio.first().map(|a| (a.sample_rate, a.channels)) else {
        return Ok(audio);
    };
    audio
        .into_iter()
        .map(|a| {
            if (a.sample_rate, a.channels) == (rate, channels) {
                return Ok(a);
            }
            let failed = |e: &dyn std::fmt::Display| format!("line `{}`: {e}", a.line_id);
            let pcm = wav::decode(&a.wav).map_err(|e| failed(&e))?;
            let pcm = with_channels(pcm.resampled(rate), channels).map_err(|e| failed(&e))?;
            Ok(LineAudio::of(a.line_id, &pcm))
        })
        .collect()
}

/// A mono take played on every channel the narration has.
fn with_channels(pcm: Pcm, channels: u16) -> Result<Pcm, String> {
    if pcm.channels == channels {
        return Ok(pcm);
    }
    if pcm.channels != 1 {
        return Err(format!(
            "the take has {} channels and the narration {channels}",
            pcm.channels
        ));
    }
    Ok(Pcm {
        samples: pcm
            .samples
            .iter()
            .flat_map(|&s| std::iter::repeat_n(s, channels as usize))
            .collect(),
        channels,
        ..pcm
    })
}

/// A `fit-line` line at the tempo the timeline gives it, as long as the
/// timeline says (docs/design.md#led-by-the-picture); any other as it is.
/// The cache keeps the voice at its own pace.
fn fitted(timeline: &Timeline, audio: &LineAudio) -> Result<(Vec<u8>, u64), String> {
    let narration = timeline
        .entries
        .iter()
        .filter_map(|e| e.narration.as_ref())
        .find(|n| n.line == audio.line_id);
    let Some((tempo, n)) = narration.and_then(|n| n.tempo_permille.map(|t| (t, n))) else {
        return Ok((audio.wav.clone(), audio.duration_ms));
    };
    let bytes =
        teleprompt_voice::stretch::fit_wav(&audio.wav, tempo.permille(), n.duration_ms.ms())
            .map_err(|e| format!("line `{}`: {e}", audio.line_id))?;
    Ok((bytes, n.duration_ms.ms()))
}

/// The duration the manifest will publish for `line_id`, read off the
/// timeline because that is what `manifest::build` copies. Deriving it from
/// the audio would compare a value against itself. `None`: the line has no
/// narration entry, so `build` drops it and publishes nothing.
fn published_duration_ms(timeline: &Timeline, line_id: &LineId) -> Option<u64> {
    timeline
        .entries
        .iter()
        .filter_map(|e| e.narration.as_ref())
        .find(|n| n.line == *line_id)
        .map(|n| n.duration_ms.ms())
}

/// `Some(message)` when a line's audio is not the length the manifest is
/// about to publish for it: the audio and the number describing it came
/// from two places. The message names both values, because which one is
/// wrong is the whole question.
fn length_mismatch(line_id: &LineId, actual_ms: u64, published_ms: u64) -> Option<String> {
    if actual_ms == published_ms {
        return None;
    }
    Some(format!(
        "line `{line_id}`: rendered audio is {actual_ms}ms but the manifest \
         publishes {published_ms}ms; a consumer placing this file at its stated \
         duration would clip or pad it"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> LineId {
        LineId::new(s)
    }

    #[test]
    fn matching_lengths_pass_the_guard() {
        assert_eq!(length_mismatch(&id("welcome"), 3250, 3250), None);
    }

    /// A manifest publishing 3250ms beside a 6500ms file: the guard names
    /// the line and both numbers.
    #[test]
    fn a_mismatch_names_the_line_and_both_lengths() {
        let msg = length_mismatch(&id("welcome"), 6500, 3250).expect("must be caught");
        assert!(msg.contains("welcome"), "{msg}");
        assert!(msg.contains("6500ms"), "{msg}");
        assert!(msg.contains("3250ms"), "{msg}");
    }

    #[test]
    fn a_mono_take_plays_on_every_channel() {
        let pcm = Pcm {
            samples: vec![1, 2],
            sample_rate: 8000,
            channels: 1,
        };
        assert_eq!(with_channels(pcm, 2).unwrap().samples, vec![1, 1, 2, 2]);
    }
}
