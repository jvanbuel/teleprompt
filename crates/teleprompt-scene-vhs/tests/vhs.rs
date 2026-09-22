use teleprompt_core::SourceSpan;
use teleprompt_scene::{BlockSource, BodyOrigin, Measured, SceneCompiler, Validated};
use teleprompt_scene_vhs::VhsScene;

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

/// The single cue of a body that has no marks in it.
fn only_span(body: &str) -> teleprompt_scene::Cue {
    let v = validated(body);
    let mut cues = VhsScene.cues(&v, "b").expect("a validated body has cues");
    assert_eq!(cues.len(), 1, "this helper is for single-cue bodies");
    cues.remove(0)
}

/// Total estimate of a whole body, as the compiler accumulates it: one
/// `estimate` per cue, summed.
fn total_ms(body: &str) -> u64 {
    let v = validated(body);
    VhsScene
        .cues(&v, "b")
        .expect("cues should split a validated body")
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
    // Command 2 of demo.tape, not the fence's offset applied to the script.
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
    let cues = VhsScene.cues(&v, "deploy").expect("cues");

    assert_eq!(cues.len(), 2);
    assert_eq!(cues[0].id, "deploy#0");
    assert_eq!(cues[1].id, "deploy#1");
}

#[test]
fn a_chunk_of_only_comments_is_not_a_span() {
    let v = validated("Sleep 1s\n# mark\n# just a note\n\n# mark\nSleep 1s\n");
    let cues = VhsScene.cues(&v, "b").expect("cues");

    assert_eq!(cues.len(), 2, "the comment-only chunk should not survive");
    // Indices stay dense after the empty chunk is dropped.
    assert_eq!(cues[1].index, 1);
}

#[test]
fn a_plain_comment_is_not_a_mark() {
    let v = validated("Sleep 1s\n# marking things\nSleep 1s\n");

    assert_eq!(VhsScene.cues(&v, "b").expect("cues").len(), 1);
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

/// The regression the cue preamble exists for: a setting established before
/// a mark still governs the cues after it.
#[test]
fn a_setting_carries_across_a_mark() {
    assert_eq!(
        total_ms("Set TypingSpeed 10ms\nType \"abc\"\n# mark\nType \"abc\"\n"),
        60,
        "the second cue should type at 10ms, not fall back to the 50ms default"
    );
}

/// The other half of carrying settings: they land in the hash, so changing
/// one invalidates the later cues whose timing it moves.
#[test]
fn changing_a_setting_changes_the_hash_of_a_later_span() {
    let a = VhsScene
        .cues(
            &validated("Set TypingSpeed 10ms\n# mark\nType \"abc\"\n"),
            "b",
        )
        .expect("cues");
    let b = VhsScene
        .cues(
            &validated("Set TypingSpeed 20ms\n# mark\nType \"abc\"\n"),
            "b",
        )
        .expect("cues");

    assert_ne!(a[0].hash, b[0].hash);
}

/// `Set TypingSpeed` carries a duration but spends none of it, so a chunk
/// holding only settings is not a cue.
#[test]
fn a_chunk_of_only_settings_is_not_a_span() {
    let v = validated("Sleep 1s\n# mark\nSet TypingSpeed 10ms\n# mark\nType \"abc\"\n");
    let cues = VhsScene.cues(&v, "b").expect("cues");

    assert_eq!(cues.len(), 2);
    assert_eq!(
        total_ms("Sleep 1s\n# mark\nSet TypingSpeed 10ms\n# mark\nType \"abc\"\n"),
        1030,
        "the setting still governs the cue after it"
    );
}

#[test]
fn settings_alone_cost_no_time() {
    assert_eq!(total_ms("Set FontSize 32\nSet Theme \"Dracula\"\n"), 0);
}

/// A cue whose timing the tape states in full needs no measuring pass —
/// which is what lets `plan` and `diff` pace a terminal scene offline.
#[test]
fn a_tape_that_states_its_timing_estimates_exactly() {
    let v = validated("Set TypingSpeed 10ms\nType \"abc\"\nEnter\nSleep 1s\n");
    let cues = VhsScene.cues(&v, "b").expect("cues");

    assert_eq!(VhsScene.estimate(&cues[0]), Measured::Exact(1040));
}

/// `Wait` is the exception, and it is an exception per cue rather than per
/// adapter: it blocks until the prompt returns, so its length is whatever the
/// command underneath takes, and that number is nowhere in the tape.
#[test]
fn a_span_containing_wait_is_estimated_and_bounded_by_its_timeout() {
    let v = validated("Type \"cargo build\"\nWait\n# mark\nSleep 1s\n");
    let cues = VhsScene.cues(&v, "b").expect("cues");

    // 11 chars at the 50ms default, plus the 5s default timeout.
    assert_eq!(VhsScene.estimate(&cues[0]), Measured::Estimated(5550));
    assert_eq!(
        VhsScene.estimate(&cues[1]),
        Measured::Exact(1000),
        "a `Wait` in one cue says nothing about the cue beside it"
    );
}

#[test]
fn a_wait_timeout_is_settable_per_tape_and_per_command() {
    assert_eq!(total_ms("Set WaitTimeout 30s\nWait\n"), 30_000);
    assert_eq!(total_ms("Wait@2s /\\$ $/\n"), 2000);
}

#[test]
fn a_wait_scope_is_checked_rather_than_assumed() {
    assert!(VhsScene
        .validate(&src("Wait+Screen /\\$ $/\nWait+Command\n"))
        .is_ok());

    let e = VhsScene
        .validate(&src("Wait+Prompt\n"))
        .expect_err("an unknown scope should fail validation");
    assert!(e[0].message.contains("Wait+Prompt"), "{}", e[0].message);
}

/// The pass-through this adapter deliberately does not do. `Set Padding 20`
/// and `Set TypingSped 10ms` are indistinguishable to a catch-all, and the
/// second is a tape that types at a speed its author did not choose.
#[test]
fn a_misspelled_setting_is_reported_rather_than_ignored() {
    let e = VhsScene
        .validate(&src("Set TypingSped 10ms\n"))
        .expect_err("an unknown setting should fail validation");

    assert!(
        e[0].message.contains("unknown setting `TypingSped`"),
        "{}",
        e[0].message
    );
    assert!(
        e[0].help
            .as_deref()
            .unwrap_or_default()
            .contains("TypingSpeed"),
        "the help should name the setting that was meant"
    );
}

/// Sourcing another tape splices it in at run time, long after `cues` has
/// decided where the cues are.
#[test]
fn source_points_at_the_include_attribute_instead() {
    let e = VhsScene
        .validate(&src("Source other.tape\n"))
        .expect_err("`Source` should fail validation");

    assert!(e[0]
        .help
        .as_deref()
        .unwrap_or_default()
        .contains("include="));
}

/// Re-timing the finished recording would slide the narration out from under
/// the action it was scheduled against.
#[test]
fn set_playback_speed_is_rejected_and_names_the_policy_that_replaces_it() {
    let e = VhsScene
        .validate(&src("Set PlaybackSpeed 2\n"))
        .expect_err("`Set PlaybackSpeed` should fail validation");

    assert!(e[0].message.contains("PlaybackSpeed"), "{}", e[0].message);
    assert!(e[0]
        .help
        .as_deref()
        .unwrap_or_default()
        .contains("stretch-action"));
}

#[test]
fn a_chord_tail_is_checked_and_a_lone_letter_is_not_a_key() {
    assert!(VhsScene.validate(&src("Ctrl+C\nAlt+Shift+Tab\n")).is_ok());

    // A chord whose tail is a typo is reported, not silently pressed; and a
    // lone character is a typo, not a keystroke.
    for bad in ["Ctrl+Etner\n", "C\n"] {
        assert!(
            VhsScene.validate(&src(bad)).is_err(),
            "{bad:?} should fail validation"
        );
    }
}

#[test]
fn an_escaped_quote_is_typed_not_counted_as_the_end_of_the_string() {
    // `say "hi"` is 8 characters; the backslashes are escapes, not keystrokes.
    assert_eq!(total_ms("Type \"say \\\"hi\\\"\"\n"), 400);

    let e = VhsScene
        .validate(&src("Type \"never closed\n"))
        .expect_err("an unterminated string should fail validation");
    assert!(e[0].message.contains("never closed"), "{}", e[0].message);
}

#[test]
fn a_trailing_argument_after_a_string_is_rejected_rather_than_swallowed() {
    let e = VhsScene
        .validate(&src("Type \"hi\" \"there\"\n"))
        .expect_err("trailing content should fail validation");

    assert!(e[0].message.contains("trailing"), "{}", e[0].message);
}

#[test]
fn a_lowercase_command_is_diagnosed_as_a_lowercase_command() {
    let e = VhsScene
        .validate(&src("type \"hi\"\n"))
        .expect_err("a lowercase command should fail validation");

    assert!(
        e[0].help
            .as_deref()
            .unwrap_or_default()
            .contains("write `Type`"),
        "a capitalisation slip should not read as a missing feature"
    );
}

/// The name `scene: terminal` resolves to, from the other side. A rename on
/// either side of that lookup is meant to fail here.
#[test]
fn the_adapter_answers_to_the_name_the_terminal_scene_resolves_to() {
    assert_eq!(VhsScene.kind(), "vhs");
    assert_eq!(
        teleprompt_core::config::default_adapter("terminal"),
        VhsScene.kind()
    );
}

/// `stretch-action` says the action fills the narration above it. Until
/// something re-times the tape, that is a number on a timeline and nothing
/// else: a capture still runs the tape at its authored pace, and the
/// difference is a frozen frame.
#[test]
fn a_stretched_span_is_re_timed_to_last_exactly_as_long_as_asked() {
    let cue = only_span(
        "Set TypingSpeed 50ms\n\
         Type \"cargo test\"\n\
         Enter\n\
         Sleep 1s\n",
    );
    let before = match VhsScene.estimate(&cue) {
        Measured::Exact(ms) => ms,
        other => panic!("a tape without Wait is exact: {other:?}"),
    };

    let retimed = VhsScene
        .retime(&cue, before * 3)
        .expect("a tape stating its own timing can be re-timed");

    let after = VhsScene.estimate(&only_span(&retimed));
    assert_eq!(
        after,
        Measured::Exact(before * 3),
        "re-timed to {}ms, got {after:?}\n{retimed}",
        before * 3
    );
}

/// Not just a pause bolted on the end: the whole action spreads out, which
/// is what "the typing slows to fill the sentence above it" means. A tape
/// that types at its old speed and then sits still for twelve seconds is
/// the frozen frame again, wearing a Sleep.
#[test]
fn stretching_slows_the_typing_rather_than_only_padding_the_end() {
    let cue = only_span("Set TypingSpeed 50ms\nType \"hello\"\nSleep 500ms\n");
    let retimed = VhsScene.retime(&cue, 3_000).expect("re-timable");

    let speed: u64 = retimed
        .lines()
        .find_map(|l| l.strip_prefix("Set TypingSpeed "))
        .and_then(|v| v.trim().trim_end_matches("ms").parse().ok())
        .expect("the re-timed tape states a typing speed");
    assert!(
        speed > 50,
        "typing still runs at the authored speed: {retimed}"
    );
    let sleep: u64 = retimed
        .lines()
        .find_map(|l| l.strip_prefix("Sleep "))
        .and_then(|v| v.trim().trim_end_matches("ms").parse().ok())
        .expect("the re-timed tape keeps its sleep");
    assert!(sleep > 500, "the sleep did not stretch: {retimed}");
}

/// A cue whose length the tape does not state cannot be promised to any
/// duration. `Wait` blocks until a prompt returns, and no amount of
/// arithmetic here changes how long `cargo build` takes.
#[test]
fn a_span_that_waits_cannot_be_re_timed() {
    let cue = only_span("Type \"cargo build\"\nEnter\nWait\n");
    assert_eq!(VhsScene.retime(&cue, 30_000), None);
}

/// Down as well as up: `trim-action` asks for a shorter action, and the
/// same arithmetic runs in reverse.
#[test]
fn a_span_can_be_re_timed_shorter_as_well_as_longer() {
    let cue = only_span("Set TypingSpeed 100ms\nType \"slow\"\nSleep 4s\n");
    let before = match VhsScene.estimate(&cue) {
        Measured::Exact(ms) => ms,
        other => panic!("{other:?}"),
    };
    let retimed = VhsScene.retime(&cue, before / 2).expect("re-timable");
    assert_eq!(
        VhsScene.estimate(&only_span(&retimed)),
        Measured::Exact(before / 2),
        "{retimed}"
    );
}

/// `Hide` stops the recording; the commands keep running. So the time they
/// take is not the cue's: a cue's duration is how long something is on
/// screen, and nothing hidden is. Counting it would size the slot for work
/// nobody sees and leave the narration waiting through it.
#[test]
fn hidden_commands_cost_the_beat_nothing() {
    let shown = total_ms("Set TypingSpeed 10ms\nType \"ls\"\nSleep 500ms\n");
    let with_setup = total_ms(
        "Set TypingSpeed 10ms\n\
         Hide\n\
         Type \"cd project\"\n\
         Enter\n\
         Sleep 2s\n\
         Show\n\
         Type \"ls\"\n\
         Sleep 500ms\n",
    );
    assert_eq!(
        with_setup, shown,
        "two seconds of hidden setup changed how long the cue lasts"
    );
}

/// And `Show` gives the time back: what follows it is on screen and counts.
#[test]
fn what_is_shown_again_counts_again() {
    let body = "Hide\nSleep 5s\nShow\nSleep 700ms\n";
    assert_eq!(total_ms(body), 700);
}

/// The environment belongs to the scene, not the tape — the same reason
/// `Set Shell` does. A scene is a session: its blocks share one shell,
/// whose environment is settled before the first of them runs.
#[test]
fn env_is_refused_and_says_where_it_goes() {
    let errors = VhsScene.validate(&src("Env FOO bar\n")).unwrap_err();
    assert!(
        errors[0].message.contains("`Env` is set by teleprompt"),
        "{:?}",
        errors[0]
    );
    assert!(
        errors[0]
            .help
            .as_deref()
            .is_some_and(|h| h.contains("scene.<name>.env")),
        "{:?}",
        errors[0]
    );
}

/// teleprompt owns what a capture writes, for the reason `Output` gives.
#[test]
fn screenshot_is_refused_like_output_is() {
    let errors = VhsScene
        .validate(&src("Screenshot shot.png\n"))
        .unwrap_err();
    assert!(
        errors[0].message.contains("written by teleprompt"),
        "{:?}",
        errors[0]
    );
}

/// The rule the adapter already applied to `Set`, now applied to commands:
/// a line `check` accepts and the capture drops is a video that is wrong
/// rather than missing. These are read, so they compile.
#[test]
fn the_commands_a_capture_honours_all_compile() {
    for body in ["Hide\nShow\n", "Require bash\n", "Copy \"text\"\nPaste\n"] {
        assert!(
            VhsScene.validate(&src(body)).is_ok(),
            "`{body}` is honoured by the capture and must compile"
        );
    }
}
