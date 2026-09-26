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
    let mut r = Resampler::new(from, to);
    let mut out = r.push(samples);
    out.extend(r.finish());
    out
}

/// [`resample`] for audio that arrives in pieces: each output sample is
/// made once every input sample it depends on has arrived, so the result
/// is the same as resampling the whole at once.
pub struct Resampler {
    step: f64,
    cutoff: f64,
    half_width: f64,
    from: u32,
    to: u32,
    /// The input still needed, starting at absolute sample `offset`.
    input: Vec<f32>,
    offset: usize,
    /// The next output sample's index.
    next: usize,
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        // Cycles per input sample.
        let cutoff = ROLL_OFF * 0.5 * from.min(to) as f64 / from as f64;
        Self {
            step: from as f64 / to as f64,
            cutoff,
            half_width: ZERO_CROSSINGS / (2.0 * cutoff),
            from,
            to,
            input: Vec::new(),
            offset: 0,
            next: 0,
        }
    }

    /// Takes the next `samples`; the output they complete.
    pub fn push(&mut self, samples: &[f32]) -> Vec<f32> {
        self.input.extend_from_slice(samples);
        let received = self.offset + self.input.len();
        let mut out = Vec::new();
        while (self.next as f64 * self.step + self.half_width).floor() < received as f64 {
            out.push(self.sample(self.next));
            self.next += 1;
        }
        self.forget();
        out
    }

    /// The output that was waiting on input that will not come.
    pub fn finish(&mut self) -> Vec<f32> {
        let received = (self.offset + self.input.len()) as u64;
        let total = (received * self.to as u64 / self.from as u64) as usize;
        let out = (self.next..total).map(|i| self.sample(i)).collect();
        self.next = total.max(self.next);
        out
    }

    fn sample(&self, i: usize) -> f32 {
        if self.from == self.to {
            return self.input.get(i - self.offset).copied().unwrap_or(0.0);
        }
        let t = i as f64 * self.step;
        let first = (t - self.half_width).ceil().max(0.0) as usize;
        let last = (t + self.half_width).floor() as usize;
        let end = self.offset + self.input.len();
        (first.max(self.offset)..=last.min(end.saturating_sub(1)))
            .map(|j| {
                let x = j as f64 - t;
                self.input[j - self.offset] as f64 * kernel(x, self.cutoff, self.half_width)
            })
            .sum::<f64>() as f32
    }

    /// Drops input no output still to come depends on.
    fn forget(&mut self) {
        let needed = (self.next as f64 * self.step - self.half_width)
            .ceil()
            .max(0.0) as usize;
        let drop = needed.saturating_sub(self.offset).min(self.input.len());
        self.input.drain(..drop);
        self.offset += drop;
    }
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
