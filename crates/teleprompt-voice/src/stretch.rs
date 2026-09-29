//! A line played faster or slower, keeping its pitch, for `fit-line`
//! (docs/design.md#led-by-the-picture).
//!
//! WSOLA: the output is built from overlapping windows of the input, each
//! taken from where the tempo puts it, give or take a few milliseconds,
//! shifted to where it best continues the window before. The shift keeps
//! the waveform's phase across the joins, so a voice neither clicks nor
//! warbles, and taking windows faster or slower than it writes them
//! changes the length and not the pitch.

use crate::Pcm;

/// Each window's length: a few pitch periods of a low voice.
const WINDOW_MS: u32 = 30;
/// How far a window may move to line up with the one before.
const SEARCH_MS: u32 = 8;

/// `pcm` at `tempo_permille` thousandths of its own pace: 1100 is ten
/// percent faster, and so ten percent shorter. Every channel is taken from
/// the same windows, so they stay together.
pub fn stretch(pcm: &Pcm, tempo_permille: u32) -> Pcm {
    let ch = usize::from(pcm.channels.max(1));
    let frames_in = pcm.samples.len() / ch;
    if tempo_permille == 1000 || tempo_permille == 0 || frames_in == 0 {
        return pcm.clone();
    }
    let tempo = f64::from(tempo_permille) / 1000.0;
    let target = (frames_in as f64 / tempo).round() as usize;
    let window = ((pcm.sample_rate * WINDOW_MS / 1000) as usize).max(4) & !1;
    let samples = if frames_in < window {
        // Too short to join windows: shorter or longer by the difference,
        // which in a few milliseconds nobody hears.
        let mut s = pcm.samples.clone();
        s.resize(target * ch, 0);
        s
    } else {
        overlap_add(pcm, ch, frames_in, target, window, tempo)
    };
    Pcm {
        sample_rate: pcm.sample_rate,
        channels: pcm.channels,
        samples,
    }
}

fn overlap_add(
    pcm: &Pcm,
    ch: usize,
    frames_in: usize,
    target: usize,
    window: usize,
    tempo: f64,
) -> Vec<i16> {
    let hop = window / 2;
    let search = (pcm.sample_rate * SEARCH_MS / 1000) as usize;
    // Past the end reads as silence, so the last windows need no special case.
    let at = |frame: usize, c: usize| -> f32 {
        if frame < frames_in {
            f32::from(pcm.samples[frame * ch + c])
        } else {
            0.0
        }
    };
    let mono: Vec<f32> = (0..frames_in + window + search)
        .map(|f| (0..ch).map(|c| at(f, c)).sum::<f32>() / ch as f32)
        .collect();
    // A periodic Hann window: two of them, half a window apart, sum to one.
    let hann: Vec<f32> = (0..window)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / window as f32).cos())
        .collect();

    let mut out = vec![0.0f32; (target + window) * ch];
    let mut previous = 0usize;
    let mut k = 0usize;
    while k * hop < target {
        let written = k * hop;
        let chosen = if k == 0 {
            0
        } else {
            let nominal = (written as f64 * tempo).round() as usize;
            best_match(&mono, previous + hop, nominal, hop, search)
        };
        for i in 0..window {
            // The first window has nothing before it to fade in against.
            let w = if k == 0 && i < hop { 1.0 } else { hann[i] };
            for c in 0..ch {
                out[(written + i) * ch + c] += w * at(chosen + i, c);
            }
        }
        previous = chosen;
        k += 1;
    }
    out.truncate(target * ch);
    out.into_iter()
        .map(|s| s.round().clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16)
        .collect()
}

/// Where, within `search` of `nominal`, the next window best continues the
/// input from `natural`: the offset whose first `hop` frames correlate most
/// with what follows the window before.
fn best_match(mono: &[f32], natural: usize, nominal: usize, hop: usize, search: usize) -> usize {
    let from = nominal.saturating_sub(search);
    let to = (nominal + search).min(mono.len().saturating_sub(hop));
    let reference = &mono[natural.min(mono.len() - hop)..][..hop];
    (from..=to.max(from))
        .max_by(|&a, &b| {
            let score = |s: usize| -> f32 {
                mono[s..][..hop]
                    .iter()
                    .zip(reference)
                    .map(|(x, y)| x * y)
                    .sum()
            };
            score(a).total_cmp(&score(b))
        })
        .unwrap_or(nominal)
}

/// [`stretch`], then padded or trimmed by the fraction of a millisecond a
/// tempo in whole thousandths leaves, to last exactly `ms`: the length the
/// timeline and manifest publish for it.
pub fn stretch_to(pcm: &Pcm, tempo_permille: u32, ms: u64) -> Pcm {
    let mut out = stretch(pcm, tempo_permille);
    let rate = u64::from(out.sample_rate.max(1));
    // The fewest frames that reach `ms` whole milliseconds.
    let frames = (ms * rate).div_ceil(1000) as usize;
    out.samples
        .resize(frames * usize::from(out.channels.max(1)), 0);
    out
}

/// [`stretch_to`] on a WAV file's bytes, as `dub` writes a `fit-line` line
/// and the preview serves it.
pub fn fit_wav(bytes: &[u8], tempo_permille: u32, ms: u64) -> Result<Vec<u8>, String> {
    let pcm = crate::wav::decode(bytes)?;
    Ok(crate::wav::encode(&stretch_to(&pcm, tempo_permille, ms)))
}
