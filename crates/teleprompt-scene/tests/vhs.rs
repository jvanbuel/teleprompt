use teleprompt_core::SourceSpan;
use teleprompt_scene::{
    BlockSource, BodyOrigin, Measured, SceneCompiler, SceneRegistry, Validated, VhsScene,
};

/// A fence opening on line 1. Body line `i` is therefore at absolute line `1 + i + 1`.
const SPAN: SourceSpan = SourceSpan {
    line: 1,
    column: 1,
    len: 0,
};

fn src(body: &str) -> BlockSource {
    BlockSource {
        scene: "terminal".into(),
        body: body.into(),
        origin: BodyOrigin::Inline { fence: SPAN },
    }
}

fn included_src(path: &str, body: &str) -> BlockSource {
    BlockSource {
        scene: "terminal".into(),
        body: body.into(),
        origin: BodyOrigin::Included { path: path.into() },
    }
}

fn validated(body: &str) -> Validated {
    VhsScene
        .validate(&src(body))
        .expect("test body should validate")
}

/// Total estimate of a whole body, as the compiler accumulates it: one
/// `estimate` per span, summed.
fn total_ms(body: &str) -> u64 {
    let v = validated(body);
    VhsScene
        .spans(&v, "b")
        .expect("spans should split a validated body")
        .iter()
        .filter_map(|s| VhsScene.estimate(s).duration_ms())
        .sum()
}

#[test]
fn a_realistic_tape_validates() {
    assert!(VhsScene
        .validate(&src("Set FontSize 32\n\
             Set TypingSpeed 40ms\n\
             Type \"acme deploy --prod\"\n\
             Enter\n\
             Sleep 3s\n",))
        .is_ok());
}

#[test]
fn an_unknown_command_is_an_error_naming_the_line() {
    let e = VhsScene
        .validate(&src("Sleep 1s\nTyp \"oops\"\n"))
        .expect_err("an unknown command should fail validation");

    assert_eq!(e.len(), 1);
    assert!(e[0].message.contains("Typ"), "{}", e[0].message);
    // Body line 1 sits at script line 1 + 1 + 1.
    assert_eq!(e[0].span.expect("diagnostic should carry a span").line, 3);
    assert_eq!(e[0].file, None);
}

#[test]
fn an_included_body_reports_its_own_file_and_line() {
    let e = VhsScene
        .validate(&included_src("demo.tape", "Sleep 1s\nnope\n"))
        .expect_err("an unknown command should fail validation");

    assert_eq!(e[0].file.as_deref(), Some("demo.tape"));
    // Line 2 of demo.tape, not the fence's offset applied to the script.
    assert_eq!(e[0].span.expect("diagnostic should carry a span").line, 2);
}

#[test]
fn output_is_rejected_because_teleprompt_owns_framing() {
    let e = VhsScene
        .validate(&src("Output demo.gif\n"))
        .expect_err("`Output` should fail validation");

    assert!(e[0].message.contains("Output"), "{}", e[0].message);
}

/// The other half of §7.5's capture conflict: teleprompt drives its own PTY,
/// so the tape does not get to choose the shell.
#[test]
fn set_shell_is_rejected_alongside_output() {
    let e = VhsScene
        .validate(&src("Set Shell \"fish\"\n"))
        .expect_err("`Set Shell` should fail validation");

    assert!(e[0].message.contains("Set Shell"), "{}", e[0].message);
}

#[test]
fn every_bad_line_is_reported_not_just_the_first() {
    let e = VhsScene
        .validate(&src("nope\nSleep\nEnter three\n"))
        .expect_err("three bad lines should fail validation");

    assert_eq!(e.len(), 3);
}

#[test]
fn marks_split_a_tape_into_one_span_per_beat() {
    let v = validated("Type \"a\"\n# mark\nSleep 1s\n");
    let spans = VhsScene.spans(&v, "deploy").expect("spans");

    assert_eq!(spans.len(), 2);
    assert_eq!(spans[0].id, "deploy#0");
    assert_eq!(spans[1].id, "deploy#1");
}

#[test]
fn a_chunk_of_only_comments_is_not_a_span() {
    let v = validated("Sleep 1s\n# mark\n# just a note\n\n# mark\nSleep 1s\n");
    let spans = VhsScene.spans(&v, "b").expect("spans");

    assert_eq!(spans.len(), 2, "the comment-only chunk should not survive");
    // Indices stay dense after the empty chunk is dropped.
    assert_eq!(spans[1].index, 1);
}

#[test]
fn a_plain_comment_is_not_a_mark() {
    let v = validated("Sleep 1s\n# marking things\nSleep 1s\n");

    assert_eq!(VhsScene.spans(&v, "b").expect("spans").len(), 1);
}

#[test]
fn sleeps_and_keystrokes_add_up() {
    // 3 chars at the 50ms default, one Enter at 50ms, plus 2s.
    assert_eq!(total_ms("Type \"abc\"\nEnter\nSleep 2s\n"), 2200);
}

#[test]
fn typing_speed_applies_to_typing_and_keys() {
    assert_eq!(
        total_ms("Set TypingSpeed 10ms\nType \"abc\"\nEnter\n"),
        40,
        "3 chars + 1 key at 10ms"
    );
}

#[test]
fn a_repeat_count_multiplies_the_keystroke() {
    assert_eq!(total_ms("Enter 3\n"), 150);
}

#[test]
fn a_per_command_speed_overrides_the_tape_setting() {
    assert_eq!(
        total_ms("Set TypingSpeed 10ms\nType@100ms \"abc\"\n"),
        300,
        "the `@` override wins for that line only"
    );
}

/// The regression the span preamble exists for: a setting established before
/// a mark still governs the beats after it.
#[test]
fn a_setting_carries_across_a_mark() {
    assert_eq!(
        total_ms("Set TypingSpeed 10ms\nType \"abc\"\n# mark\nType \"abc\"\n"),
        60,
        "the second beat should type at 10ms, not fall back to the 50ms default"
    );
}

/// The other half of carrying settings: they land in the hash, so changing
/// one invalidates the later spans whose timing it moves.
#[test]
fn changing_a_setting_changes_the_hash_of_a_later_span() {
    let a = VhsScene
        .spans(
            &validated("Set TypingSpeed 10ms\n# mark\nType \"abc\"\n"),
            "b",
        )
        .expect("spans");
    let b = VhsScene
        .spans(
            &validated("Set TypingSpeed 20ms\n# mark\nType \"abc\"\n"),
            "b",
        )
        .expect("spans");

    assert_ne!(a[0].hash, b[0].hash);
}

/// `Set TypingSpeed` carries a duration but spends none of it, so a chunk
/// holding only settings is not a beat.
#[test]
fn a_chunk_of_only_settings_is_not_a_span() {
    let v = validated("Sleep 1s\n# mark\nSet TypingSpeed 10ms\n# mark\nType \"abc\"\n");
    let spans = VhsScene.spans(&v, "b").expect("spans");

    assert_eq!(spans.len(), 2);
    assert_eq!(
        total_ms("Sleep 1s\n# mark\nSet TypingSpeed 10ms\n# mark\nType \"abc\"\n"),
        1030,
        "the setting still governs the beat after it"
    );
}

#[test]
fn settings_alone_cost_no_time() {
    assert_eq!(total_ms("Set FontSize 32\nSet Theme \"Dracula\"\n"), 0);
}

/// A tape's own runtime is not in the tape, so the number is a starting
/// point for M1's measuring pass rather than a promise.
#[test]
fn a_tape_estimate_is_never_exact() {
    let v = validated("Sleep 1s\n");
    let spans = VhsScene.spans(&v, "b").expect("spans");

    assert_eq!(VhsScene.estimate(&spans[0]), Measured::Estimated(1000));
}

#[test]
fn the_registry_serves_vhs_as_a_builtin() {
    let r = SceneRegistry::with_builtins();

    assert_eq!(r.get("vhs").map(SceneCompiler::kind), Some("vhs"));
    assert!(r.available().contains(&"vhs"));
}
