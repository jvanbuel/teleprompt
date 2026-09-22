//! A recording, cut into one asciicast per span.
//!
//! asciicast v2 because that is what renders: a JSON header and one line
//! per chunk of output, which `agg` turns into frames without a browser, a
//! display or a font server.

use std::path::Path;

/// Write the asciicast for `[from_ms, to_ms)` of `events`.
///
/// Two things are folded in rather than dropped.
///
/// Everything before `from_ms` is replayed as one chunk at t=0. Terminal
/// output is cumulative — a span opens on the screen the spans before it
/// left behind — so replaying the prefix reconstructs that screen exactly
/// without spending any of the span's own time on it.
///
/// A `Hide`den stretch is the same idea inside the span. The commands ran
/// and the screen changed; nobody was meant to watch it happen. So the
/// output arrives at the instant the stretch began and the stretch's time
/// is closed up, which is what a recording that was paused looks like.
pub fn write(
    path: &Path,
    events: &[(u64, Vec<u8>)],
    hidden: &[(u64, u64)],
    cols: u16,
    rows: u16,
    from_ms: u64,
    to_ms: u64,
) -> std::io::Result<()> {
    let mut out = String::new();
    out.push_str(&format!(
        "{}\n",
        serde_json::json!({
            "version": 2,
            "width": cols,
            "height": rows,
            "env": { "TERM": "xterm-256color" },
        })
    ));

    let prior: Vec<u8> = events
        .iter()
        .filter(|(at, _)| *at < from_ms)
        .flat_map(|(_, bytes)| bytes.clone())
        .collect();
    if !prior.is_empty() {
        out.push_str(&format!("{}\n", event(0, &prior)));
    }
    for (at, bytes) in events
        .iter()
        .filter(|(at, _)| *at >= from_ms && *at < to_ms)
    {
        out.push_str(&format!(
            "{}\n",
            event(shown_at(*at, hidden, from_ms), bytes)
        ));
    }

    std::fs::write(path, out)
}

/// When an event is seen, once the hidden stretches are closed up.
fn shown_at(at: u64, hidden: &[(u64, u64)], from_ms: u64) -> u64 {
    // An event inside a hidden stretch is seen at the moment that stretch
    // began: by the time the recording resumes, its effect is simply part
    // of the screen.
    let moment = hidden
        .iter()
        .find(|(start, end)| at >= *start && at < *end)
        .map_or(at, |(start, _)| *start);

    // Everything hidden before that moment is time the recording did not
    // spend, clamped to this span so a stretch in an earlier one is not
    // subtracted twice.
    let closed: u64 = hidden
        .iter()
        .filter(|(start, end)| *end > from_ms && *start < moment)
        .map(|(start, end)| (*end).min(moment).saturating_sub((*start).max(from_ms)))
        .sum();

    moment.saturating_sub(from_ms).saturating_sub(closed)
}

fn event(at_ms: u64, bytes: &[u8]) -> String {
    serde_json::json!([at_ms as f64 / 1000.0, "o", String::from_utf8_lossy(bytes),]).to_string()
}

#[cfg(test)]
mod tests {
    use super::shown_at;

    /// Nothing hidden: an event is seen when it happened, relative to the
    /// span.
    #[test]
    fn without_hiding_time_is_time() {
        assert_eq!(shown_at(1_500, &[], 1_000), 500);
    }

    /// A hidden stretch closes up: what follows it arrives earlier by
    /// exactly as long as it lasted.
    #[test]
    fn a_hidden_stretch_is_closed_up() {
        let hidden = [(1_200, 1_700)];
        assert_eq!(shown_at(1_100, &hidden, 1_000), 100, "before it: unmoved");
        assert_eq!(
            shown_at(1_400, &hidden, 1_000),
            200,
            "inside it: seen where it began"
        );
        assert_eq!(
            shown_at(1_900, &hidden, 1_000),
            400,
            "after it: 900ms in, less the 500ms nobody watched"
        );
    }

    /// A stretch hidden in an earlier span is already part of the screen
    /// this one opens on, so it is not subtracted again here.
    #[test]
    fn a_stretch_hidden_before_this_span_does_not_move_it() {
        let hidden = [(200, 700)];
        assert_eq!(shown_at(1_500, &hidden, 1_000), 500);
    }
}
