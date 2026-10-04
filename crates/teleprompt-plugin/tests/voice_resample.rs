//! Changing a recording's sample rate: the microphone's to the
//! recognizer's, and a take's to the rate the rest of the narration has.

use teleprompt_plugin::voice::resample;

fn sine(hz: f64, rate: u32, seconds: f64) -> Vec<f32> {
    let n = (rate as f64 * seconds) as usize;
    (0..n)
        .map(|i| (0.5 * (2.0 * std::f64::consts::PI * hz * i as f64 / rate as f64).sin()) as f32)
        .collect()
}

fn rms(s: &[f32]) -> f64 {
    (s.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / s.len() as f64).sqrt()
}

#[test]
fn the_same_rate_changes_nothing() {
    let s = sine(440.0, 16_000, 0.1);
    assert_eq!(resample(&s, 16_000, 16_000), s);
}

#[test]
fn the_length_scales_with_the_rate() {
    let s = sine(440.0, 48_000, 1.0);
    assert_eq!(resample(&s, 48_000, 16_000).len(), 16_000);
    assert_eq!(resample(&s, 48_000, 44_100).len(), 44_100);
    assert_eq!(resample(&s, 16_000, 24_000).len(), 72_000);
}

/// A tone keeps its pitch and its loudness, up or down.
#[test]
fn a_tone_comes_out_as_the_same_tone() {
    for (from, to) in [(48_000, 16_000), (44_100, 24_000), (16_000, 48_000)] {
        let out = resample(&sine(440.0, from, 1.0), from, to);
        let want = sine(440.0, to, 1.0);
        // The edges lack neighbours on one side; judge the middle.
        let (a, b) = (out.len() / 10, out.len() * 9 / 10);
        let err: Vec<f32> = out[a..b]
            .iter()
            .zip(&want[a..b])
            .map(|(x, y)| x - y)
            .collect();
        assert!(rms(&err) < 0.01, "{from} → {to}: error {}", rms(&err));
    }
}

/// What the new rate cannot hold is filtered out rather than folded back
/// down as a tone that was never said.
#[test]
fn what_the_new_rate_cannot_hold_is_removed_not_aliased() {
    let out = resample(&sine(10_000.0, 48_000, 1.0), 48_000, 16_000);
    let (a, b) = (out.len() / 10, out.len() * 9 / 10);
    assert!(rms(&out[a..b]) < 0.02, "rms {}", rms(&out[a..b]));
}

/// A take at the microphone's rate joins narration at another rate without
/// its length moving by a millisecond, since the manifest publishes it.
#[test]
fn resampled_audio_keeps_its_length_to_the_millisecond() {
    for (from, to, ms) in [
        (44_100, 24_000, 1234),
        (48_000, 44_100, 1001),
        (16_000, 48_000, 7),
    ] {
        let pcm = teleprompt_plugin::voice::Pcm {
            sample_rate: from,
            channels: 1,
            samples: vec![100; (ms * from as u64 / 1000) as usize],
        };
        let before = pcm.duration_ms();
        let after = pcm.resampled(to);
        assert_eq!(after.sample_rate, to);
        assert_eq!(after.duration_ms(), before, "{from} → {to}");
    }
}

/// Audio that arrives in chunks, as a microphone's does, comes out as it
/// would all at once: no seam at the edges of the chunks.
#[test]
fn resampling_in_chunks_is_resampling_all_at_once() {
    let s = sine(440.0, 48_000, 1.0);
    for (from, to) in [(48_000, 16_000), (44_100, 16_000), (16_000, 48_000)] {
        let whole = resample(&s, from, to);
        let mut streaming = teleprompt_plugin::voice::Resampler::new(from, to);
        let mut pieces = Vec::new();
        for chunk in s.chunks(4_801) {
            pieces.extend(streaming.push(chunk));
        }
        pieces.extend(streaming.finish());
        assert_eq!(pieces.len(), whole.len(), "{from} → {to}");
        let worst = pieces
            .iter()
            .zip(&whole)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(worst < 1e-6, "{from} → {to}: off by {worst}");
    }
}
