use std::collections::BTreeMap;
use std::ops::Range;

use crate::Position;

/// When, in a take's samples, the follower heard each line begin and end,
/// and so where to cut the take into lines.
pub struct TakeLog {
    /// The line the take started on.
    from: usize,
    /// The sample at which each line's first word had been heard.
    begun: BTreeMap<usize, usize>,
    /// The sample at which each line had been heard to the end.
    finished: BTreeMap<usize, usize>,
}

/// How far before a line's end was reported the break after it may lie: a
/// report comes after the last word, by the recognizer's lag and the word.
const LOOKBACK_MS: usize = 1500;

/// The length of the frames loudness is measured over.
const FRAME_MS: usize = 10;

/// Silence kept on either side of a line's speech, so a soft first or last
/// sound is not clipped.
const MARGIN_MS: usize = 50;

impl TakeLog {
    pub fn new(from: usize) -> Self {
        Self {
            from,
            begun: BTreeMap::new(),
            finished: BTreeMap::new(),
        }
    }

    /// The follower placed the reader at `at` once the take's first
    /// `sample` samples had been heard.
    pub fn heard(&mut self, at: Position, sample: usize) {
        for line in self.from..=at.line {
            if line < at.line || at.word > 0 {
                self.begun.entry(line).or_insert(sample);
            }
            if line < at.line {
                self.finished.entry(line).or_insert(sample);
            }
        }
    }

    /// The lines read in full, each with the part of `audio` that holds it.
    ///
    /// A line is read in full when it was heard to its end, and not skipped:
    /// a skipped line is heard to begin at the same moment as the one after
    /// it, since the follower jumped past it in one step.
    pub fn lines(&self, audio: &[f32], rate: u32) -> Vec<(usize, Range<usize>)> {
        let samples = |ms: usize| ms * rate as usize / 1000;
        let loudness = Loudness::of(audio, samples(FRAME_MS));
        let lookback = samples(LOOKBACK_MS);
        let mut out = Vec::new();
        for (&line, &end) in &self.finished {
            let Some(&begun) = self.begun.get(&line) else {
                continue;
            };
            if self.begun.get(&(line + 1)) == Some(&begun) {
                continue;
            }
            // Nobody reads before a take starts, so its first line starts
            // with it; a pause inside that line is not a break.
            let start = match line.checked_sub(1).and_then(|l| self.finished.get(&l)) {
                Some(&previous) if line > self.from => {
                    loudness.break_in(previous.saturating_sub(lookback)..begun)
                }
                _ => 0,
            };
            let after = self.begun.get(&(line + 1)).copied().unwrap_or(audio.len());
            let stop = loudness.break_in(end.saturating_sub(lookback)..after);
            if let Some(speech) = loudness.trim(start..stop, samples(MARGIN_MS)) {
                out.push((line, speech));
            }
        }
        out
    }
}

/// A take's loudness, frame by frame.
struct Loudness {
    frame: usize,
    rms: Vec<f32>,
    /// Below this, a frame is silence.
    quiet: f32,
    len: usize,
}

impl Loudness {
    fn of(audio: &[f32], frame: usize) -> Self {
        let frame = frame.max(1);
        let rms: Vec<f32> = audio
            .chunks(frame)
            .map(|c| (c.iter().map(|x| x * x).sum::<f32>() / c.len() as f32).sqrt())
            .collect();
        let mut sorted = rms.clone();
        sorted.sort_by(f32::total_cmp);
        let pick = |q: f32| sorted.get((q * sorted.len() as f32) as usize).copied();
        let floor = pick(0.1).unwrap_or(0.0);
        let level = pick(0.95).unwrap_or(0.0);
        Self {
            frame,
            quiet: floor + 0.1 * (level - floor),
            rms,
            len: audio.len(),
        }
    }

    fn is_quiet(&self, f: usize) -> bool {
        self.rms[f] <= self.quiet
    }

    /// The middle of the longest silence within `within`, in samples; the
    /// quietest frame when nothing there is silent.
    fn break_in(&self, within: Range<usize>) -> usize {
        let first = (within.start / self.frame).min(self.rms.len());
        let last = within.end.div_ceil(self.frame).min(self.rms.len());
        let mut best: Option<Range<usize>> = None;
        let mut run = first;
        for f in first..=last {
            if f < last && self.is_quiet(f) {
                continue;
            }
            if f > run && best.as_ref().is_none_or(|b| f - run > b.len()) {
                best = Some(run..f);
            }
            run = f + 1;
        }
        let at = match best {
            Some(b) => (b.start + b.end) * self.frame / 2,
            None => (first..last)
                .min_by(|&a, &b| self.rms[a].total_cmp(&self.rms[b]))
                .map_or(within.start, |f| f * self.frame + self.frame / 2),
        };
        at.min(self.len)
    }

    /// `span` without the silence at either end, bar `margin`; `None` when
    /// it holds nothing but silence.
    fn trim(&self, span: Range<usize>, margin: usize) -> Option<Range<usize>> {
        let frames = span.start / self.frame..span.end.div_ceil(self.frame).min(self.rms.len());
        let first = frames.clone().find(|&f| !self.is_quiet(f))?;
        let last = frames.rev().find(|&f| !self.is_quiet(f))?;
        let start = (first * self.frame).saturating_sub(margin).max(span.start);
        let end = ((last + 1) * self.frame + margin).min(span.end);
        Some(start..end)
    }
}
