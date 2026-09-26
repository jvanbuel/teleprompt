//! Band-limited resampling: each output sample is the input convolved with
//! a windowed sinc whose cutoff is the lower rate's Nyquist, so going down
//! filters out what the new rate cannot hold instead of aliasing it.

use std::f64::consts::PI;

/// Zero crossings of the sinc on each side of a sample: the filter's
/// sharpness against its cost.
const ZERO_CROSSINGS: f64 = 16.0;

/// The cutoff, as a fraction of the lower rate's Nyquist, leaving the
/// window room to roll off before it.
const ROLL_OFF: f64 = 0.95;

/// Mono `samples` at `from` Hz, at `to` Hz.
pub fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.is_empty() {
        return samples.to_vec();
    }
    let out_len = (samples.len() as u64 * to as u64 / from as u64) as usize;
    let step = from as f64 / to as f64;
    // Cycles per input sample.
    let cutoff = ROLL_OFF * 0.5 * from.min(to) as f64 / from as f64;
    let half_width = ZERO_CROSSINGS / (2.0 * cutoff);
    (0..out_len)
        .map(|i| {
            let t = i as f64 * step;
            let first = (t - half_width).ceil().max(0.0) as usize;
            let last = ((t + half_width).floor() as usize).min(samples.len() - 1);
            (first..=last)
                .map(|j| {
                    let x = j as f64 - t;
                    samples[j] as f64 * kernel(x, cutoff, half_width)
                })
                .sum::<f64>() as f32
        })
        .collect()
}

/// A low-pass sinc at `cutoff`, under a Blackman window `half_width` wide.
fn kernel(x: f64, cutoff: f64, half_width: f64) -> f64 {
    let sinc = if x == 0.0 {
        1.0
    } else {
        (2.0 * PI * cutoff * x).sin() / (2.0 * PI * cutoff * x)
    };
    let w = x / half_width;
    let window = 0.42 + 0.5 * (PI * w).cos() + 0.08 * (2.0 * PI * w).cos();
    2.0 * cutoff * sinc * window
}
