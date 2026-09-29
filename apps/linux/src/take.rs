//! Where a take is: what the record button, the tally, the clock and the
//! microphone all follow. One value, so a take cannot be paused and
//! sending at once, or counting down while it runs.

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Take {
    /// No take: `ran` is how long the last one ran, for the clock.
    Idle { ran: Duration },
    /// Counting down to a take.
    Counting,
    /// Audio is sent: since `since`, after `before` from before a pause.
    Running { since: Instant, before: Duration },
    /// Under way, sending nothing.
    Paused { before: Duration },
}

impl Default for Take {
    fn default() -> Self {
        Self::Idle {
            ran: Duration::ZERO,
        }
    }
}

impl Take {
    /// Counting down, or under way: another take waits.
    pub fn is_taking(&self) -> bool {
        !matches!(self, Self::Idle { .. })
    }

    pub fn is_counting(&self) -> bool {
        matches!(self, Self::Counting)
    }

    /// Running or paused: there is a take to keep.
    pub fn is_under_way(&self) -> bool {
        matches!(self, Self::Running { .. } | Self::Paused { .. })
    }

    pub fn is_paused(&self) -> bool {
        matches!(self, Self::Paused { .. })
    }

    /// Audio is being sent.
    pub fn is_sending(&self) -> bool {
        matches!(self, Self::Running { .. })
    }

    /// Starts a countdown, unless a take is already on; whether it did.
    pub fn count(&mut self) -> bool {
        let idle = !self.is_taking();
        if idle {
            *self = Self::Counting;
        }
        idle
    }

    /// The take starts, its clock at zero.
    pub fn start(&mut self, now: Instant) {
        *self = Self::Running {
            since: now,
            before: Duration::ZERO,
        };
    }

    /// Pauses a running take or resumes a paused one: `Some(paused)`, or
    /// `None` when there is no take under way.
    pub fn toggle_pause(&mut self, now: Instant) -> Option<bool> {
        match *self {
            Self::Running { since, before } => {
                *self = Self::Paused {
                    before: before + now.saturating_duration_since(since),
                };
                Some(true)
            }
            Self::Paused { before } => {
                *self = Self::Running { since: now, before };
                Some(false)
            }
            _ => None,
        }
    }

    /// Ends the take, or the countdown; the clock keeps what it ran.
    pub fn stop(&mut self, now: Instant) {
        *self = Self::Idle {
            ran: self.elapsed(now),
        };
    }

    /// A new script or session: no take, and the clock back at zero.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// How long the take has run, pauses left out.
    pub fn elapsed(&self, now: Instant) -> Duration {
        match *self {
            Self::Idle { ran } => ran,
            Self::Counting => Duration::ZERO,
            Self::Running { since, before } => before + now.saturating_duration_since(since),
            Self::Paused { before } => before,
        }
    }
}
