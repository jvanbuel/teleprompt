mod common;

use common::{example, json};
use teleprompt_gtk::api::{encode_samples, ClientMessage, Position, Script, ServerMessage};

#[test]
fn the_script_example_reads() {
    let script: Script = serde_json::from_str(&example("script.json")).unwrap();
    let ids: Vec<&str> = script.lines.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(ids, ["welcome", "deploy"]);
    assert_eq!(script.lines[0].words().count(), 8);
    assert!(!script.lines[0].stale);
    assert_eq!(script.lines[0].said, None);
    let at: Vec<Position> = script.shots.iter().map(|s| s.at).collect();
    let pos = |line, word| Position { line, word };
    assert_eq!(at, [pos(0, 0), pos(0, 3), pos(1, 0)]);
    assert!(script.shots[0].clip.is_none());
    assert!(script.shots[1]
        .clip
        .as_deref()
        .unwrap()
        .starts_with("/api/v1/clips/"));
}

#[test]
fn the_server_message_examples_read() {
    assert_eq!(
        ServerMessage::parse(&example("reached.json")).unwrap(),
        ServerMessage::Reached {
            at: Position { line: 1, word: 0 },
            play: vec!["welcome-b#0".into()]
        }
    );
    assert_eq!(
        ServerMessage::parse(&example("stopped.json")).unwrap(),
        ServerMessage::Stopped {
            saved: vec!["welcome".into()]
        }
    );
    assert_eq!(
        ServerMessage::parse(&example("error.json")).unwrap(),
        ServerMessage::Error(r#"not a message this server knows: {"type":"rewind"}"#.into())
    );
}

/// A later v1 server may send more; the client ignores what it does not
/// know rather than failing.
#[test]
fn what_this_client_does_not_know_is_ignored() {
    let newer = r#"{"type":"reached","line":2,"word":1,"play":[],"confidence":0.9}"#;
    assert_eq!(
        ServerMessage::parse(newer).unwrap(),
        ServerMessage::Reached {
            at: Position { line: 2, word: 1 },
            play: vec![]
        }
    );
    assert_eq!(
        ServerMessage::parse(r#"{"type":"level","rms":0.2}"#).unwrap(),
        ServerMessage::Unknown("level".into())
    );
}

#[test]
fn commands_are_the_examples() {
    let start = ClientMessage::Start {
        from: 1,
        rate: 48_000,
    };
    assert_eq!(json(&start.json()), json(&example("start.json")));
    assert_eq!(
        json(&ClientMessage::Stop.json()),
        json(&example("stop.json"))
    );
}

#[test]
fn samples_are_little_endian_floats() {
    assert_eq!(
        encode_samples(&[1.0, -0.5]),
        [0x00, 0x00, 0x80, 0x3F, 0x00, 0x00, 0x00, 0xBF]
    );
}
