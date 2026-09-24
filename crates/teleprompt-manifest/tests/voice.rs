//! A line's voice tier as the manifest publishes it: the tier requested,
//! the tier delivered and, only when they differ, why.

use teleprompt_core::Hash;
use teleprompt_manifest::LineEntry;

fn line_json(requested: &str, actual: &str, reason: &str) -> String {
    let hash = serde_json::to_string(&Hash::of(b"")).unwrap();
    format!(
        r#"{{"id":"a","text":"Hi.","chapter":"intro","start_ms":0,"duration_ms":1000,"duration_source":"measured","audio":"audio/a.wav","voice_source":"{requested}","voice_source_actual":"{actual}","downgrade_reason":{reason},"source_hash":{hash},"audio_hash":{hash}}}"#
    )
}

#[test]
fn a_consistent_line_reads_and_writes_back_byte_for_byte() {
    for json in [
        line_json("synthetic", "synthetic", "null"),
        line_json("recorded", "synthetic", r#""no takes recorded""#),
    ] {
        let line: LineEntry = serde_json::from_str(&json).expect(&json);
        assert_eq!(serde_json::to_string(&line).unwrap(), json);
    }
}

/// Three fields could say three inconsistent things; a manifest that does is
/// refused on reading rather than carried along.
#[test]
fn an_inconsistent_voice_tier_is_refused() {
    for (requested, actual, reason) in [
        ("synthetic", "synthetic", r#""a reason for nothing""#),
        ("recorded", "synthetic", "null"),
        ("synthetic", "recorded", r#""an upgrade""#),
    ] {
        let json = line_json(requested, actual, reason);
        assert!(serde_json::from_str::<LineEntry>(&json).is_err(), "{json}");
    }
}
