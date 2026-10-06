//! How a long step says how far it got, on stderr: a line for a person, or
//! with JSON progress on, an event an app reads.

use std::sync::atomic::{AtomicBool, Ordering};

static JSON: AtomicBool = AtomicBool::new(false);

/// Says progress as JSON events from now on, or as lines: set once, from
/// the command's `--format`.
pub fn set_json(json: bool) {
    JSON.store(json, Ordering::SeqCst);
}

/// Whether progress is said as JSON events, for an app reading them.
pub fn json() -> bool {
    JSON.load(Ordering::SeqCst)
}

/// One step of a long command: the line `human` says, or as JSON an event
/// `{"event": "progress", "stage": …, …fields}`, one per line.
pub fn progress(stage: &str, human: impl FnOnce() -> String, fields: serde_json::Value) {
    if json() {
        let mut event = serde_json::json!({ "event": "progress", "stage": stage });
        if let (Some(e), serde_json::Value::Object(f)) = (event.as_object_mut(), fields) {
            e.extend(f);
        }
        eprintln!("{event}");
    } else {
        eprintln!("{}", human());
    }
}

/// One shot of a capture recorded, as [`progress`] says it.
pub fn capture_progress(p: teleprompt_plugin::capture::Progress) {
    progress(
        "capture",
        || format!("  [{}/{}] {} {}", p.done, p.of, p.scene, p.shot),
        serde_json::json!({ "done": p.done, "of": p.of, "scene": p.scene, "shot": p.shot }),
    );
}
