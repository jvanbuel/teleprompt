use teleprompt_core::{BlockId, SourceSpan};
use teleprompt_plugin::scene::{
    BlockSource, BodyOrigin, Measured, MockScene, SceneCompiler, SceneRegistry,
};

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
        origin: BodyOrigin::Inline { fence: SPAN },
    }
}

/// A body loaded with `include=`: its lines belong to that file and are
/// numbered from 1 there, not offset by the fence's position in the script.
fn included_src(path: &str, body: &str) -> BlockSource {
    BlockSource {
        scene: "mock".into(),
        body: body.into(),
        origin: BodyOrigin::Included { path: path.into() },
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
fn marks_split_the_block_into_shots() {
    let m = MockScene;
    let v = m.validate(&src("wait 500ms\nmark\nwait 300ms\n")).unwrap();
    let shots = m.shots(&v, &BlockId::from("a-1-a")).unwrap();
    assert_eq!(shots.len(), 2);
    assert_eq!(shots[0].id, "a-1-a#0");
    assert_eq!(shots[1].id, "a-1-a#1");
}

#[test]
fn a_block_with_no_marks_is_one_shot() {
    let m = MockScene;
    let v = m.validate(&src("wait 500ms\nwait 300ms\n")).unwrap();
    assert_eq!(m.shots(&v, &BlockId::from("b")).unwrap().len(), 1);
}

#[test]
fn estimate_sums_the_waits_exactly() {
    let m = MockScene;
    let v = m.validate(&src("wait 500ms\nwait 1s\n")).unwrap();
    let shots = m.shots(&v, &BlockId::from("b")).unwrap();
    assert_eq!(m.estimate(&shots[0]), Measured::Exact(1500));
}

#[test]
fn shot_hashes_differ_by_content_and_repeat_for_identical_content() {
    let m = MockScene;
    let v1 = m.validate(&src("wait 500ms\nmark\nwait 500ms\n")).unwrap();
    let s1 = m.shots(&v1, &BlockId::from("b")).unwrap();
    assert_eq!(s1[0].hash, s1[1].hash, "identical shots hash identically");

    let v2 = m.validate(&src("wait 501ms\n")).unwrap();
    let s2 = m.shots(&v2, &BlockId::from("b")).unwrap();
    assert_ne!(s1[0].hash, s2[0].hash);
}

#[test]
fn registry_resolves_builtin_plugins_and_rejects_unknown_ones() {
    let r = SceneRegistry::with_builtins();
    assert_eq!(r.get("mock").map(|a| a.kind()), Some("mock"));
    assert!(r.get("playwright").is_none(), "not a builtin");
}

#[test]
fn a_body_ending_with_mark_yields_one_shot_not_two() {
    let m = MockScene;
    let v = m.validate(&src("wait 500ms\nmark\n")).unwrap();
    let shots = m.shots(&v, &BlockId::from("c")).unwrap();
    assert_eq!(shots.len(), 1, "trailing mark does not create empty shot");
    assert_eq!(shots[0].index, 0, "surviving shot has contiguous index 0");
}

#[test]
fn a_body_starting_with_mark_yields_one_shot_not_two() {
    let m = MockScene;
    let v = m.validate(&src("mark\nwait 500ms\n")).unwrap();
    let shots = m.shots(&v, &BlockId::from("d")).unwrap();
    assert_eq!(shots.len(), 1, "leading mark does not create empty shot");
    assert_eq!(shots[0].index, 0, "surviving shot has contiguous index 0");
}

#[test]
fn consecutive_marks_yield_contiguous_shots() {
    let m = MockScene;
    let v = m
        .validate(&src("wait 100ms\nmark\nmark\nwait 200ms\n"))
        .unwrap();
    let shots = m.shots(&v, &BlockId::from("e")).unwrap();
    assert_eq!(
        shots.len(),
        2,
        "two consecutive marks create two shots, not three"
    );
    assert_eq!(shots[0].index, 0, "first shot has index 0");
    assert_eq!(shots[1].index, 1, "second shot has index 1 (contiguous)");
}

#[test]
fn a_body_that_is_only_mark_yields_zero_shots() {
    let m = MockScene;
    let v = m.validate(&src("mark\n")).unwrap();
    let shots = m.shots(&v, &BlockId::from("f")).unwrap();
    assert_eq!(shots.len(), 0, "all-mark body yields no shots");
}

/// At the contract's own level: an included body's lines are numbered from 1 in
/// the file they came from, and the diagnostic names that file rather than the
/// script the caller renders against.
#[test]
fn an_included_bodys_diagnostic_names_its_own_file_and_line() {
    let m = MockScene;
    let e = m
        .validate(&included_src(
            "scripts/steps.mock",
            "wait 100ms\nmark\nbogus directive here\n",
        ))
        .unwrap_err();
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].span.unwrap().line, 3, "line 3 of the included file");
    assert_eq!(e[0].file.as_deref(), Some("scripts/steps.mock"));
    assert!(e[0]
        .render("scripts/h1.md")
        .contains("scripts/steps.mock:3:1"));
}

/// An inline body carries no file override: the script the caller is
/// rendering against is the right answer, and the fence offset still applies.
#[test]
fn an_inline_bodys_diagnostic_defers_to_the_callers_file() {
    let m = MockScene;
    let e = m
        .validate(&src("wait 500ms\nclick everything\n"))
        .unwrap_err();
    assert_eq!(e[0].file, None);
    assert!(e[0].render("scripts/h1.md").contains("scripts/h1.md:3:1"));
}

// ---------------------------------------------------------------------------
// `validate` and `estimate` disagreed about what content is. `validate` split
// on whitespace; `estimate` used `strip_prefix("wait ")`, so a `check`-clean
// script could silently lose five seconds.
// ---------------------------------------------------------------------------

fn only_shot(body: &str) -> teleprompt_plugin::scene::Shot {
    let m = MockScene;
    let v = m.validate(&src(body)).expect("body must validate");
    let shots = m.shots(&v, &BlockId::from("b")).unwrap();
    assert_eq!(shots.len(), 1, "fixture is meant to be a single shot");
    shots.into_iter().next().unwrap()
}

/// `wait<TAB>5000ms` passed `check` and then contributed 0 ms.
#[test]
fn a_tab_between_wait_and_its_duration_is_counted() {
    let m = MockScene;
    assert_eq!(
        m.estimate(&only_shot("wait\t5000ms\n")),
        Measured::Exact(5000),
        "check accepts a tab, so the estimate has to as well"
    );
}

/// `wait 5000ms and then some` passed `check` and then contributed 0 ms.
/// Silently ignoring the tail is what let the two halves disagree, so the
/// tail is now rejected outright.
#[test]
fn trailing_garbage_after_a_duration_is_rejected_by_validate() {
    let m = MockScene;
    let e = m.validate(&src("wait 5000ms and then some\n")).unwrap_err();
    assert_eq!(e.len(), 1);
    assert!(
        e[0].message.contains("trailing `and`"),
        "the diagnostic must name what it did not understand: {}",
        e[0].message
    );
}

#[test]
fn trailing_garbage_after_mark_is_rejected_by_validate() {
    let m = MockScene;
    let e = m.validate(&src("mark now\n")).unwrap_err();
    assert_eq!(e.len(), 1);
    assert!(e[0].message.contains("`mark` takes no arguments"));
}

/// Ruling F13's wording said "whitespace-only" when it meant "no executable
/// content", so a chunk of nothing but `#` comments between two `mark`s
/// survived the filter and became a phantom zero-duration shot — exactly the
/// shape F13 was written to eliminate.
#[test]
fn a_comment_only_chunk_between_marks_produces_no_shot() {
    let m = MockScene;
    let v = m
        .validate(&src(
            "wait 100ms\nmark\n# just a comment\nmark\nwait 200ms\n",
        ))
        .unwrap();
    let shots = m.shots(&v, &BlockId::from("g")).unwrap();
    assert_eq!(
        shots.len(),
        2,
        "the comment-only chunk is empty, not a zero-duration shot: {:?}",
        shots.iter().map(|s| &s.source).collect::<Vec<_>>()
    );
    assert_eq!(shots[0].index, 0);
    assert_eq!(shots[1].index, 1, "indices stay contiguous");
    assert_eq!(m.estimate(&shots[0]), Measured::Exact(100));
    assert_eq!(m.estimate(&shots[1]), Measured::Exact(200));
}

/// The property the three fixes above are really about: anything `validate`
/// accepts as a `wait`, `estimate` counts.
#[test]
fn everything_validate_accepts_as_a_wait_is_counted_by_estimate() {
    for (body, expected) in [
        ("wait 500ms\n", 500),
        ("wait\t500ms\n", 500),
        ("   wait    500ms   \n", 500),
        ("wait 1s\n", 1000),
        ("# comment\nwait 250ms\n\nwait 250ms\n", 500),
    ] {
        assert_eq!(
            MockScene.estimate(&only_shot(body)),
            Measured::Exact(expected),
            "body {body:?}"
        );
    }
}
