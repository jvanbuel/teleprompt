//! Durations: how long an author said something takes ([`DurationMs`]) and
//! where a scheduled duration came from ([`DurationSource`]). Shared by the
//! scheduler, the timeline and the manifest, so it lives in `core`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DurationSource {
    Exact,
    Estimated,
    Measured,
    /// The plugin cannot say (a Playwright script). Neither an estimate nor
    /// zero: the scheduler gives the shot its line's length.
    Unknown,
}

impl DurationSource {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Estimated => "estimated",
            Self::Measured => "measured",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for DurationSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// A duration an author states — a pause, a lead-in, a transition — in
/// milliseconds, and never more than a day ([`DurationMs::MAX`]). Longer is
/// a typo, and every way to make one checks, so sums of these stay far
/// from `u64::MAX` however many a script has.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct DurationMs(u64);

impl DurationMs {
    pub const ZERO: Self = Self(0);
    pub const MAX: Self = Self(24 * 60 * 60 * 1000);

    /// For constants: fails to compile, or panics, past [`Self::MAX`].
    pub const fn millis(ms: u64) -> Self {
        assert!(ms <= Self::MAX.0, "a DurationMs is at most a day");
        Self(ms)
    }

    pub fn new(ms: u64) -> Result<Self, String> {
        if ms > Self::MAX.0 {
            return Err(format!(
                "{ms}ms is longer than a day, the most a duration may be"
            ));
        }
        Ok(Self(ms))
    }

    /// `250ms`, `1s`, `1.5s`, or a bare integer taken as milliseconds.
    pub fn parse(v: &str) -> Result<Self, String> {
        let err = || format!("`{v}` is not a duration (try `250ms` or `1s`)");
        let ms = if let Some(n) = v.strip_suffix("ms") {
            n.trim().parse::<u64>().map_err(|_| err())?
        } else if let Some(n) = v.strip_suffix('s') {
            let secs: f64 = n.trim().parse().map_err(|_| err())?;
            if secs < 0.0 {
                return Err(err());
            }
            crate::time::ms_from_seconds(secs)
        } else {
            v.parse::<u64>().map_err(|_| err())?
        };
        Self::new(ms).map_err(|_| format!("`{v}` is longer than a day, the most a duration may be"))
    }

    pub const fn ms(self) -> u64 {
        self.0
    }
}

impl<'de> Deserialize<'de> for DurationMs {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::new(u64::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}
