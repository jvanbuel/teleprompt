use std::sync::Arc;
use teleprompt_core::{LineId, SourceSpan};
use teleprompt_voice::backends::Backends;
use teleprompt_voice::VoiceRegistry;

mod stub;
use stub::StubVoice;

fn backends(ids: &[&'static str]) -> Backends {
    let mut r = VoiceRegistry::default();
    for id in ids {
        r.register(Arc::new(StubVoice::new(id)));
    }
    Backends::from_registry(r)
}

fn at(line: usize) -> SourceSpan {
    SourceSpan {
        line,
        column: 1,
        len: 1,
    }
}

#[test]
fn the_narrator_is_in_the_cast_though_no_line_names_it() {
    let (a, b) = (LineId::from("a"), LineId::from("b"));
    let cast = backends(&["narrator", "guest"])
        .cast("narrator", [("guest", &a, at(1)), ("guest", &b, at(2))])
        .expect("both backends exist");
    assert_eq!(cast.narrator().id(), "narrator");
    assert_eq!(cast.ids().collect::<Vec<_>>(), ["guest", "narrator"]);
    assert_eq!(
        cast.versions().keys().collect::<Vec<_>>(),
        ["guest", "narrator"]
    );
    assert!(cast.get("other").is_none());
}

#[test]
fn a_missing_backend_is_one_error_naming_every_line_at_the_first() {
    let (a, b) = (LineId::from("a"), LineId::from("b"));
    let errors = backends(&["narrator"])
        .cast("narrator", [("nope", &a, at(3)), ("nope", &b, at(7))])
        .err()
        .expect("`nope` is not a backend");
    assert_eq!(errors.len(), 1);
    assert!(
        errors[0]
            .message
            .starts_with("lines `a`, `b`: unknown voice backend `nope`"),
        "{}",
        errors[0].message
    );
    assert_eq!(errors[0].span, Some(at(3)));
}

#[test]
fn an_unusable_narrator_is_reported_alone() {
    let a = LineId::from("a");
    let errors = backends(&["narrator"])
        .cast("gone", [("nope", &a, at(1))])
        .err()
        .expect("`gone` is not a backend");
    assert_eq!(errors.len(), 1);
    assert!(
        errors[0]
            .message
            .starts_with("unknown voice backend `gone`"),
        "{}",
        errors[0].message
    );
}
