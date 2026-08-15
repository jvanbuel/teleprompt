use teleprompt_core::SourceSpan;
use teleprompt_scene::{BlockSource, Measured, MockScene, SceneCompiler, SceneRegistry};

/// A fence opening on line 1. Body line `i` is therefore at absolute line `1 + i + 1`.
const SPAN: SourceSpan = SourceSpan {
    line: 1,
    column: 1,
    len: 0,
};

fn src(body: &str) -> BlockSource {
    BlockSource {
        scene: "mock".into(),
        body: body.into(),
        span: SPAN,
    }
}

#[test]
fn valid_body_passes_validation() {
    let m = MockScene;
    assert!(m.validate(&src("wait 500ms\nmark\nwait 1s\n")).is_ok());
}

#[test]
fn unknown_directive_is_an_error_naming_the_line() {
    let m = MockScene;
    let e = m
        .validate(&src("wait 500ms\nclick everything\n"))
        .unwrap_err();
    assert_eq!(e.len(), 1);
    assert!(e[0].message.contains("unknown mock directive `click`"));
    assert_eq!(e[0].span.unwrap().line, 3);
}

#[test]
fn comments_and_blank_lines_are_ignored() {
    let m = MockScene;
    assert!(m.validate(&src("# setup\n\nwait 100ms\n")).is_ok());
}

#[test]
fn marks_split_the_block_into_spans() {
    let m = MockScene;
    let v = m.validate(&src("wait 500ms\nmark\nwait 300ms\n")).unwrap();
    let spans = m.spans(&v, "a-1-a").unwrap();
    assert_eq!(spans.len(), 2);
    assert_eq!(spans[0].id, "a-1-a#0");
    assert_eq!(spans[1].id, "a-1-a#1");
}

#[test]
fn a_block_with_no_marks_is_one_span() {
    let m = MockScene;
    let v = m.validate(&src("wait 500ms\nwait 300ms\n")).unwrap();
    assert_eq!(m.spans(&v, "b").unwrap().len(), 1);
}

#[test]
fn estimate_sums_the_waits_exactly() {
    let m = MockScene;
    let v = m.validate(&src("wait 500ms\nwait 1s\n")).unwrap();
    let spans = m.spans(&v, "b").unwrap();
    assert_eq!(m.estimate(&spans[0]), Measured::Exact(1500));
}

#[test]
fn span_hashes_differ_by_content_and_repeat_for_identical_content() {
    let m = MockScene;
    let v1 = m.validate(&src("wait 500ms\nmark\nwait 500ms\n")).unwrap();
    let s1 = m.spans(&v1, "b").unwrap();
    assert_eq!(s1[0].hash, s1[1].hash, "identical spans hash identically");

    let v2 = m.validate(&src("wait 501ms\n")).unwrap();
    let s2 = m.spans(&v2, "b").unwrap();
    assert_ne!(s1[0].hash, s2[0].hash);
}

#[test]
fn registry_resolves_builtin_adapters_and_rejects_unknown_ones() {
    let r = SceneRegistry::with_builtins();
    assert_eq!(r.get("mock").map(|a| a.kind()), Some("mock"));
    assert!(r.get("playwright").is_none(), "not available in M0");
}
