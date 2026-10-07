//! Milliseconds, the unit everything is scheduled in: moments and lengths
//! kept apart, and in the other units the tools around it speak.

use serde::{Deserialize, Serialize};

/// `ms` as seconds to the millisecond, `S.mmm`: what ffmpeg's `-t` and `-ss`
/// take, written exactly rather than through a float.
pub fn ffmpeg_seconds(ms: u64) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

/// The number of frames `ms` spans at `fps`, rounded to the nearest.
pub fn frames(ms: u64, fps: u32) -> u64 {
    ms.saturating_mul(u64::from(fps)).saturating_add(500) / 1000
}

/// Seconds, as a file or a server states them, in whole milliseconds. A
/// negative or unreadable (NaN) figure is none, and an absurdly large one
/// the most there is: float-to-integer casts saturate.
pub fn ms_from_seconds(seconds: f64) -> u64 {
    (seconds * 1000.0).round() as u64
}

/// `ms` for a person: seconds to one decimal, as `4.2s`.
pub fn short(ms: u64) -> String {
    format!("{:.1}s", ms as f64 / 1000.0)
}

/// A moment in the video: milliseconds from its start.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct TimeMs(u64);

/// A length of time, in milliseconds: how long something lasts, never
/// where it is. A moment and a length add to a moment; two moments do not
/// add, and their difference is a length.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct SpanMs(u64);

impl TimeMs {
    pub const ZERO: Self = Self(0);

    pub const fn at(ms: u64) -> Self {
        Self(ms)
    }

    pub const fn ms(self) -> u64 {
        self.0
    }

    /// How long after `earlier` this is; zero if it is not after it.
    pub fn since(self, earlier: TimeMs) -> SpanMs {
        SpanMs(self.0.saturating_sub(earlier.0))
    }
}

impl SpanMs {
    pub const ZERO: Self = Self(0);

    pub const fn of(ms: u64) -> Self {
        Self(ms)
    }

    pub const fn ms(self) -> u64 {
        self.0
    }
}

/// Saturating: a scheduled time that saturates is visibly absurd, one that
/// wraps quietly wrong.
impl std::ops::Add<SpanMs> for TimeMs {
    type Output = TimeMs;
    fn add(self, span: SpanMs) -> TimeMs {
        TimeMs(self.0.saturating_add(span.0))
    }
}

impl std::ops::Add for SpanMs {
    type Output = SpanMs;
    fn add(self, other: SpanMs) -> SpanMs {
        SpanMs(self.0.saturating_add(other.0))
    }
}

impl std::ops::Sub<SpanMs> for TimeMs {
    type Output = TimeMs;
    fn sub(self, span: SpanMs) -> TimeMs {
        TimeMs(self.0.saturating_sub(span.0))
    }
}

impl std::ops::Sub for SpanMs {
    type Output = SpanMs;
    fn sub(self, other: SpanMs) -> SpanMs {
        SpanMs(self.0.saturating_sub(other.0))
    }
}

impl std::ops::Sub for TimeMs {
    type Output = SpanMs;
    fn sub(self, earlier: TimeMs) -> SpanMs {
        self.since(earlier)
    }
}

impl From<crate::DurationMs> for SpanMs {
    fn from(d: crate::DurationMs) -> Self {
        Self(d.ms())
    }
}

/// A `fit-line` line's playing speed, in thousandths: 1150 is 15% faster.
/// Never zero, and never 1000, which is no change at all and is written as
/// no tempo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct Tempo(u32);

impl Tempo {
    /// The tempo `permille` makes, or `None` for 1000 (unchanged) or 0.
    pub fn new(permille: u32) -> Option<Self> {
        (permille != 0 && permille != 1000).then_some(Self(permille))
    }

    pub fn permille(self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for Tempo {
    type Error = String;
    fn try_from(permille: u32) -> Result<Self, String> {
        Self::new(permille).ok_or_else(|| format!("a tempo of {permille}‰ is no tempo: omit it"))
    }
}

impl From<Tempo> for u32 {
    fn from(t: Tempo) -> u32 {
        t.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        assert_eq!(ffmpeg_seconds(4_050), "4.050");
        assert_eq!(ffmpeg_seconds(7), "0.007");
        assert_eq!(frames(1_000, 30), 30);
        assert_eq!(frames(16, 30), 0);
        assert_eq!(frames(17, 30), 1);
        assert_eq!(ms_from_seconds(1.2345), 1_235);
        assert_eq!(short(4_249), "4.2s");
    }

    #[test]
    fn a_moment_and_a_length_make_a_moment() {
        let start = TimeMs::at(1_000);
        let end = start + SpanMs::of(500);
        assert_eq!(end, TimeMs::at(1_500));
        assert_eq!(end - start, SpanMs::of(500));
        assert_eq!(start - end, SpanMs::ZERO);
        assert_eq!(end - SpanMs::of(500), start);
        assert_eq!(SpanMs::of(500) - SpanMs::of(200), SpanMs::of(300));
        assert_eq!(TimeMs::at(u64::MAX) + SpanMs::of(1), TimeMs::at(u64::MAX));
    }

    #[test]
    fn a_tempo_is_a_change() {
        assert_eq!(Tempo::new(1000), None);
        assert_eq!(Tempo::new(0), None);
        assert_eq!(Tempo::new(1150).map(Tempo::permille), Some(1150));
        assert!(serde_json::from_str::<Tempo>("1000").is_err());
        assert_eq!(
            serde_json::to_string(&Tempo::new(900).unwrap()).unwrap(),
            "900"
        );
    }
}
