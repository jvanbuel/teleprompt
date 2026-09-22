//! A recording, cut into one asciicast per span.
//!
//! asciicast v2 because that is what renders: a JSON header and one line
//! per chunk of output, which `agg` turns into frames without a browser,
//! a display or a font server.

use std::path::Path;

/// Write the asciicast for `[from_ms, to_ms)` of `events`.
///
/// Everything before `from_ms` is replayed as one chunk at t=0. Terminal
/// output is cumulative — a span opens on the screen the spans before it
/// left behind — so replaying the prefix reconstructs that screen exactly
/// without spending any of the span's own time on it.
pub fn write(
    path: &Path,
    events: &[(u64, Vec<u8>)],
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
        out.push_str(&format!("{}\n", event(at - from_ms, bytes)));
    }

    std::fs::write(path, out)
}

fn event(at_ms: u64, bytes: &[u8]) -> String {
    serde_json::json!([at_ms as f64 / 1000.0, "o", String::from_utf8_lossy(bytes),]).to_string()
}
