//! Milliseconds, the unit everything is scheduled in, in the other units
//! the tools around it speak.

/// `ms` as seconds to the millisecond, `S.mmm`: what ffmpeg's `-t` and `-ss`
/// take, written exactly rather than through a float.
pub fn ffmpeg_seconds(ms: u64) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

/// The number of frames `ms` spans at `fps`, rounded to the nearest.
pub fn frames(ms: u64, fps: u32) -> u64 {
    (ms * u64::from(fps) + 500) / 1000
}

/// Seconds, as a file or a server states them, in whole milliseconds.
pub fn ms_from_seconds(seconds: f64) -> u64 {
    (seconds * 1000.0).round() as u64
}

/// `ms` for a person: seconds to one decimal, as `4.2s`.
pub fn short(ms: u64) -> String {
    format!("{:.1}s", ms as f64 / 1000.0)
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
}
