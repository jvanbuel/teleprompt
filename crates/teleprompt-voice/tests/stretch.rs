//! A line played faster or slower for `fit-line`, keeping its pitch
//! (docs/design.md#led-by-the-picture).

use teleprompt_voice::stretch::stretch;
use teleprompt_voice::Pcm;

const RATE: u32 = 24_000;

fn tone(hz: f64, seconds: f64, channels: u16) -> Pcm {
    let frames = (f64::from(RATE) * seconds) as usize;
    let mut samples = Vec::with_capacity(frames * channels as usize);
    for i in 0..frames {
        let v = (8000.0 * (std::f64::consts::TAU * hz * i as f64 / f64::from(RATE)).sin()) as i16;
        for _ in 0..channels {
            samples.push(v);
        }
    }
    Pcm {
        sample_rate: RATE,
        channels,
        samples,
    }
}

fn frames(pcm: &Pcm) -> usize {
    pcm.samples.len() / pcm.channels as usize
}

/// The first channel's pitch, from how often it crosses zero upwards, over
/// its middle: the ends are where the window ramps.
fn pitch_hz(pcm: &Pcm) -> f64 {
    let ch = pcm.channels as usize;
    let mono: Vec<i16> = pcm.samples.iter().step_by(ch).copied().collect();
    let (from, to) = (mono.len() / 10, mono.len() * 9 / 10);
    let crossings = mono[from..to]
        .windows(2)
        .filter(|w| w[0] < 0 && w[1] >= 0)
        .count();
    crossings as f64 * f64::from(pcm.sample_rate) / (to - from) as f64
}

/// The largest step between neighbouring samples of the first channel: a
/// click is a step a smooth tone never takes.
fn largest_step(pcm: &Pcm) -> i32 {
    let ch = pcm.channels as usize;
    let mono: Vec<i32> = pcm
        .samples
        .iter()
        .step_by(ch)
        .map(|&s| i32::from(s))
        .collect();
    mono.windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .max()
        .unwrap_or(0)
}

#[test]
fn a_faster_or_slower_line_is_that_much_shorter_or_longer() {
    let line = tone(220.0, 2.0, 1);
    for tempo in [900u32, 1080, 1150] {
        let out = stretch(&line, tempo);
        let want = (frames(&line) as f64 * 1000.0 / f64::from(tempo)).round() as usize;
        assert_eq!(frames(&out), want, "tempo {tempo}");
        assert_eq!(out.sample_rate, RATE);
    }
}

#[test]
fn its_pitch_is_kept() {
    let line = tone(440.0, 2.0, 1);
    for tempo in [900u32, 1150] {
        let hz = pitch_hz(&stretch(&line, tempo));
        assert!(
            (hz - 440.0).abs() < 440.0 * 0.02,
            "tempo {tempo}: {hz:.1} Hz"
        );
    }
}

#[test]
fn it_does_not_click() {
    let line = tone(330.0, 1.5, 1);
    let smooth = largest_step(&line);
    for tempo in [900u32, 1150] {
        let step = largest_step(&stretch(&line, tempo));
        assert!(
            step <= smooth * 3 / 2,
            "tempo {tempo}: {step} against {smooth}"
        );
    }
}

#[test]
fn every_channel_is_kept() {
    let line = tone(440.0, 1.0, 2);
    let out = stretch(&line, 1100);
    assert_eq!(out.channels, 2);
    assert_eq!(out.samples.len() % 2, 0);
    // Both channels carried the same tone, and still do.
    assert!(out.samples.chunks(2).all(|f| f[0] == f[1]));
}

#[test]
fn at_its_own_pace_a_line_is_untouched() {
    let line = tone(440.0, 1.0, 1);
    assert_eq!(stretch(&line, 1000), line);
}

#[test]
fn a_line_shorter_than_a_window_still_has_its_length() {
    let blip = tone(440.0, 0.01, 1);
    let out = stretch(&blip, 1150);
    let want = (frames(&blip) as f64 / 1.15).round() as usize;
    assert_eq!(frames(&out), want);
    let empty = Pcm {
        sample_rate: RATE,
        channels: 1,
        samples: vec![],
    };
    assert!(stretch(&empty, 1150).samples.is_empty());
}

/// Windows that join out of phase cancel, and a steady tone warbles: its
/// loudness, every 10 ms, stays within a few percent of the original's.
#[test]
fn a_steady_tone_stays_steady() {
    let line = tone(330.0, 1.5, 1);
    let rms =
        |s: &[i16]| (s.iter().map(|&v| f64::from(v).powi(2)).sum::<f64>() / s.len() as f64).sqrt();
    let level = rms(&line.samples);
    for tempo in [900u32, 1150] {
        let out = stretch(&line, tempo);
        let block = (RATE / 100) as usize;
        let worst = out.samples[block..out.samples.len() - block]
            .chunks(block)
            .filter(|c| c.len() == block)
            .map(|c| (rms(c) - level).abs() / level)
            .fold(0.0, f64::max);
        assert!(
            worst < 0.1,
            "tempo {tempo}: loudness off by {:.0}%",
            worst * 100.0
        );
    }
}

/// What `dub` writes: the length the manifest publishes, to the
/// millisecond, at a rate that does not divide into milliseconds.
#[test]
fn stretched_to_a_length_it_is_exactly_that_long() {
    use teleprompt_voice::stretch::stretch_to;
    let line = Pcm {
        sample_rate: 22_050,
        channels: 1,
        samples: tone(440.0, 1.3, 1).samples,
    };
    for ms in [1130u64, 1131, 1447] {
        assert_eq!(stretch_to(&line, 1150, ms).duration_ms(), ms);
    }
}
