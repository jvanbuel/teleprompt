use teleprompt_voice::estimator::DurationEstimator;
use teleprompt_voice::SynthRequest;

/// The rate both `WpmEstimator` and `NullVoice` default to.
///
/// One constant, not two literals, because the whole reason `null` is useful
/// as a reference backend is that its audio is *exactly* the default
/// estimate: several tests read as healthy only because the two agree, and
/// nothing but this shared definition stops them from drifting apart.
pub const DEFAULT_WPM: f64 = 150.0;

const COMMA_MS: u64 = 150;
const CLAUSE_MS: u64 = 250;
const SENTENCE_MS: u64 = 350;

/// Words per minute plus punctuation pauses. Crude, deterministic, and free
/// — which is what the inner loop needs.
pub struct WpmEstimator {
    pub wpm: f64,
}

impl Default for WpmEstimator {
    fn default() -> Self {
        Self { wpm: DEFAULT_WPM }
    }
}

/// Free function so `NullVoice` can share it without owning an estimator.
pub fn estimate_ms(text: &str, wpm: f64, speed: f64) -> u64 {
    let words = text
        .split_whitespace()
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .count() as f64;
    if words == 0.0 {
        return 0;
    }
    let speech = words / wpm * 60_000.0;
    let pauses: u64 = text
        .chars()
        .map(|c| match c {
            ',' => COMMA_MS,
            ':' | ';' => CLAUSE_MS,
            '.' | '!' | '?' => SENTENCE_MS,
            _ => 0,
        })
        .sum();
    ((speech + pauses as f64) / speed).round() as u64
}

impl DurationEstimator for WpmEstimator {
    fn estimate_ms(&self, req: &SynthRequest) -> u64 {
        estimate_ms(&req.text, self.wpm, req.speed)
    }
}
