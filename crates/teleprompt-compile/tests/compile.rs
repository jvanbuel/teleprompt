use std::path::Path;

use teleprompt_compile::compile;
use teleprompt_core::config::PartialConfig;
use teleprompt_core::ident::assign_ids;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::{resolve, Program};
use teleprompt_scene::SceneRegistry;
use teleprompt_voice::NullVoice;

fn program(src: &str) -> Program {
    let mut s = parse_script(src).unwrap();
    assign_ids(&mut s);
    resolve(
        &s,
        "demo.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .unwrap()
}

fn run(src: &str) -> teleprompt_compile::CompileOutput {
    compile(
        &program(src),
        &SceneRegistry::with_builtins(),
        &NullVoice::default(),
        Path::new("."),
        "0.1.0",
    )
    .unwrap()
}

// Controller ruling F2 (addendum): `transition` lives inside `output:`, per
// the design spec settled in Task 5 — a top-level `transition` key fails to
// deserialize under `deny_unknown_fields`.
const ONE_BEAT: &str = r#"---
scene: { mock: { adapter: mock } }
output:
  transition: { duration: 0ms }
---

# Intro

One two three four five six. {#welcome}

```teleprompt scene=mock
wait 500ms
```
"#;

#[test]
fn narration_and_the_following_action_form_one_beat() {
    let out = run(ONE_BEAT);
    assert_eq!(out.timeline.entries.len(), 1);
    let e = &out.timeline.entries[0];
    assert!(e.narration.is_some());
    assert!(e.action.is_some());
}

#[test]
fn narration_duration_comes_from_the_voice_backend() {
    let out = run(ONE_BEAT);
    let n = out.timeline.entries[0].narration.as_ref().unwrap();
    // 6 words at 150 wpm = 2400ms, plus 350ms for the full stop
    assert_eq!(n.duration_ms, 2750);
}

#[test]
fn action_duration_comes_from_the_scene_estimate() {
    let out = run(ONE_BEAT);
    assert_eq!(
        out.timeline.entries[0].action.as_ref().unwrap().duration_ms,
        500
    );
    assert_eq!(
        out.timeline.entries[0]
            .action
            .as_ref()
            .unwrap()
            .duration_source,
        "exact"
    );
}

#[test]
fn each_mark_after_the_first_span_becomes_its_own_beat() {
    let src = r#"---
scene: { mock: { adapter: mock } }
---

# Intro

One. {#a}

```teleprompt scene=mock
wait 100ms
mark
wait 200ms
mark
wait 300ms
```
"#;
    let out = run(src);
    assert_eq!(out.timeline.entries.len(), 3);
    assert!(out.timeline.entries[0].narration.is_some());
    assert!(out.timeline.entries[1].narration.is_none());
    assert_eq!(
        out.timeline.entries[1].action.as_ref().unwrap().duration_ms,
        200
    );
    assert_eq!(
        out.timeline.entries[2].action.as_ref().unwrap().duration_ms,
        300
    );
}

#[test]
fn a_pause_directive_becomes_a_silent_beat() {
    let src = "---\nscene: { mock: { adapter: mock } }\n---\n\n# A\n\nOne. {#a}\n\n<!-- teleprompt: pause 800ms -->\n";
    let out = run(src);
    assert_eq!(out.timeline.entries.len(), 2);
    let p = &out.timeline.entries[1];
    assert_eq!(p.duration_ms, 800);
    assert!(p.narration.is_none(), "a pause is silent");
    assert_eq!(p.action.as_ref().unwrap().scene, "pause");
}

#[test]
fn the_scene_records_both_its_name_and_its_adapter() {
    let out = run(ONE_BEAT);
    let a = out.timeline.entries[0].action.as_ref().unwrap();
    assert_eq!(a.scene, "mock");
    assert_eq!(a.adapter, "mock");
}

#[test]
fn an_unconfigured_scene_falls_back_to_its_default_adapter() {
    let src = "# A\n\nOne. {#a}\n\n```teleprompt scene=mock\nwait 100ms\n```\n";
    let out = run(src);
    assert_eq!(
        out.timeline.entries[0].action.as_ref().unwrap().adapter,
        "mock"
    );
}

#[test]
fn an_unavailable_adapter_is_a_diagnostic_not_a_panic() {
    let src = "# A\n\nOne. {#a}\n\n```teleprompt scene=browser\nawait page.goto('/');\n```\n";
    let p = program(src);
    let e = compile(
        &p,
        &SceneRegistry::with_builtins(),
        &NullVoice::default(),
        Path::new("."),
        "0.1.0",
    )
    .unwrap_err();
    assert!(e.0[0].message.contains("no adapter `playwright`"));
    assert!(e.0[0].help.as_deref().unwrap().contains("mock"));
}

#[test]
fn adapter_validation_errors_reach_the_caller() {
    let src = "# A\n\nOne. {#a}\n\n```teleprompt scene=mock\nclick everything\n```\n";
    let p = program(src);
    let e = compile(
        &p,
        &SceneRegistry::with_builtins(),
        &NullVoice::default(),
        Path::new("."),
        "0.1.0",
    )
    .unwrap_err();
    assert!(e.0[0].message.contains("unknown mock directive"));
}

#[test]
fn an_invalid_policy_is_a_diagnostic() {
    let src = "# A\n\nOne. {#a}\n\n```teleprompt scene=mock policy=sideways\nwait 1s\n```\n";
    let p = program(src);
    let e = compile(
        &p,
        &SceneRegistry::with_builtins(),
        &NullVoice::default(),
        Path::new("."),
        "0.1.0",
    )
    .unwrap_err();
    assert!(e.0[0].message.contains("unknown policy `sideways`"));
}

#[test]
fn the_null_backend_downgrades_a_recorded_request_and_says_why() {
    let src =
        "---\nscene: { mock: { adapter: mock } }\n---\n\n# A\n\nOne. {#a voice.source=recorded}\n";
    let out = run(src);
    let n = out.timeline.entries[0].narration.as_ref().unwrap();
    assert_eq!(n.voice_source, "recorded");
    assert_eq!(n.voice_source_actual, "synthetic");
    assert!(n.downgrade_reason.as_ref().unwrap().contains("no takes"));
}

#[test]
fn compilation_is_deterministic() {
    let a = run(ONE_BEAT);
    let b = run(ONE_BEAT);
    assert_eq!(
        serde_json::to_string(&a.timeline).unwrap(),
        serde_json::to_string(&b.timeline).unwrap()
    );
}

// Controller ruling F12 (second half): `Item::Action`'s `span` must be
// threaded into `BlockSource` for real, not `SourceSpan { line: 0, .. }`.
// The fence here opens on line 14 (verified against `parse_script`
// independently), so the invalid `bogus` directive on the block's second
// body line sits at absolute source line 16 (14 + 1 + 1, per the mock
// adapter's `span.line + i + 1` convention). A fabricated `line: 0` would
// instead report line 2 (0 + 1 + 1) — a small, visibly wrong number that a
// real user's editor would never scroll to.
const SPAN_SRC: &str = "# A\n\nOne. {#a}\n\n\n\n\n\n\n\n\n\n\n```teleprompt scene=mock\nwait 100ms\nbogus directive\n```\n";

#[test]
fn adapter_diagnostics_report_the_real_source_line_not_a_fabricated_zero() {
    let p = program(SPAN_SRC);
    let e = compile(
        &p,
        &SceneRegistry::with_builtins(),
        &NullVoice::default(),
        Path::new("."),
        "0.1.0",
    )
    .unwrap_err();
    assert!(e.0[0].message.contains("unknown mock directive `bogus`"));
    assert_eq!(
        e.0[0].span.unwrap().line,
        16,
        "must be the real absolute line, not the small number a fabricated \
         `SourceSpan {{ line: 0, .. }}` would have produced"
    );
}

// Fix round 1, finding 1: Task 6's F13 filter can reduce an all-`mark`
// action block to zero surviving spans. A narration must not reach across
// that empty block to pair with a later one — pairing stays scoped to the
// immediately following action item.
#[test]
fn a_narration_does_not_jump_an_empty_action_block_to_pair_with_a_later_one() {
    let src = r#"---
scene: { mock: { adapter: mock } }
---

# Intro

One. {#a}

```teleprompt scene=mock
mark
```

```teleprompt scene=mock
wait 400ms
```
"#;
    let out = run(src);
    assert_eq!(out.timeline.entries.len(), 2);
    assert!(
        out.timeline.entries[0].narration.is_some(),
        "the narration becomes its own beat"
    );
    assert!(
        out.timeline.entries[0].action.is_none(),
        "the empty block produces no beat, so nothing attaches to the narration"
    );
    assert!(
        out.timeline.entries[1].narration.is_none(),
        "the narration must not jump the empty block to reach the second one"
    );
    assert_eq!(
        out.timeline.entries[1].action.as_ref().unwrap().duration_ms,
        400
    );
}

// Regression pin: a narration followed by an all-`mark` block at the end of
// the program must still flush as a narration-only beat (this already
// worked before the fix above; pinned so it stays that way).
#[test]
fn a_narration_followed_by_an_empty_action_block_at_end_of_program_still_flushes() {
    let src = r#"---
scene: { mock: { adapter: mock } }
---

# Intro

One. {#a}

```teleprompt scene=mock
mark
```
"#;
    let out = run(src);
    assert_eq!(out.timeline.entries.len(), 1);
    assert!(out.timeline.entries[0].narration.is_some());
    assert!(out.timeline.entries[0].action.is_none());
}

// Fix round 1, finding 2: an unrecognised `voice.source` must be a
// diagnostic, not a silent coercion to `synthetic` (which would also skip
// the ladder's rejection-reason machinery entirely).
#[test]
fn an_invalid_voice_source_is_a_diagnostic_not_a_silent_synthetic() {
    let src = "# A\n\nOne. {#a voice.source=recordedd}\n";
    let p = program(src);
    let e = compile(
        &p,
        &SceneRegistry::with_builtins(),
        &NullVoice::default(),
        Path::new("."),
        "0.1.0",
    )
    .unwrap_err();
    assert!(e.0[0].message.contains("recordedd"));
}

// Final review, item 2: a `Beat` carries one `Config`, but its two halves
// resolve from different layers — the narration's from the segment's own
// attributes, the beat's from the following action block's. `compile` used
// to set `config: config.clone()` (the action block's) and read the
// narration's padding off that, so an identical segment produced different
// narration timing depending on whether an action block happened to follow
// it. Per spec §3.1 every segment is followed by an action block, so the
// broken case was the normal one.
const SEGMENT_WITH_LEAD_IN: &str = "One two three. {#a lead_in=1000ms}";

fn lead_in_fixture(with_action: bool) -> String {
    let mut s = format!(
        "---\nscene: {{ mock: {{ adapter: mock }} }}\n---\n\n# A\n\n{SEGMENT_WITH_LEAD_IN}\n"
    );
    if with_action {
        s.push_str("\n```teleprompt scene=mock\nwait 100ms\n```\n");
    }
    s
}

#[test]
fn a_segments_lead_in_survives_pairing_with_a_following_action_block() {
    let alone = run(&lead_in_fixture(false));
    let paired = run(&lead_in_fixture(true));

    let alone_n = alone.timeline.entries[0].narration.as_ref().unwrap();
    let paired_n = paired.timeline.entries[0].narration.as_ref().unwrap();

    assert_eq!(
        alone_n.start_ms, 1000,
        "the segment's own lead_in=1000ms places its narration"
    );
    assert_eq!(
        paired_n.start_ms, alone_n.start_ms,
        "the same segment must produce the same narration start whether or \
         not an action block follows"
    );

    // The beat's own duration differs by exactly the action block's 100ms
    // under `hold`, and by nothing else: the 850ms that used to vanish with
    // the discarded lead-in is gone.
    assert_eq!(alone.timeline.entries[0].duration_ms, 2700);
    assert_eq!(paired.timeline.entries[0].duration_ms, 2800);
}
