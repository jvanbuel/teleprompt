//! A `plan --format json` built from the scheduler's own types, so a fixture
//! cannot drift from what `teleprompt plan` writes.

use teleprompt_core::config::TransitionKind;
use teleprompt_core::{DurationSource, Hash, ItemId, LineId, PolicyKind, ShotId, SpanMs, TimeMs};
pub use teleprompt_schedule::NarrationEntry;
use teleprompt_schedule::{ActionEntry, Entry, Timeline, TransitionEntry};

/// A line: its id, start and length.
pub fn line(id: &str, start: u64, length: u64) -> NarrationEntry {
    NarrationEntry {
        line: LineId::from(id),
        source_hash: Hash::of(id.as_bytes()),
        audio_hash: Hash::of(id.as_bytes()),
        start_ms: TimeMs::at(start),
        duration_ms: SpanMs::of(length),
        duration_source: DurationSource::Measured,
        recorded: false,
        tempo_permille: None,
    }
}

/// A shot: its id, scene, start, length and where the length comes from.
pub fn shot(id: &str, scene: &str, start: u64, length: u64, source: DurationSource) -> ActionEntry {
    ActionEntry {
        shot: ShotId::from(id),
        scene: scene.into(),
        adapter: scene.into(),
        shot_hash: Hash::of(id.as_bytes()),
        capture_key: Hash::of(id.as_bytes()),
        session: None,
        start_ms: TimeMs::at(start),
        duration_ms: SpanMs::of(length),
        duration_source: source,
    }
}

/// The plan's JSON: `duration` long, one entry per line or shot pair.
pub fn json(duration: u64, entries: Vec<(Option<NarrationEntry>, Option<ActionEntry>)>) -> String {
    let entries = entries
        .into_iter()
        .map(|(narration, action)| {
            let item = match (&narration, &action) {
                (Some(n), _) => ItemId::from(n.line.clone()),
                (None, Some(a)) => ItemId::from(a.shot.clone()),
                (None, None) => panic!("an entry needs a line or a shot"),
            };
            let (start_ms, duration_ms) = narration
                .as_ref()
                .map(|n| (n.start_ms, n.duration_ms))
                .or(action.as_ref().map(|a| (a.start_ms, a.duration_ms)))
                .unwrap();
            Entry {
                item,
                start_ms,
                duration_ms,
                policy: PolicyKind::Hold,
                narration,
                action,
                transition: TransitionEntry {
                    kind: TransitionKind::Cut,
                    duration_ms: SpanMs::ZERO,
                },
            }
        })
        .collect();
    let plan = Timeline {
        version: 1,
        script: "script.md".into(),
        locale: "en".into(),
        duration_ms: SpanMs::of(duration),
        generated_by: "test".into(),
        entries,
    };
    serde_json::to_string(&plan).unwrap()
}
