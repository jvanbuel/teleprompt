use teleprompt_core::config::PartialConfig;
use teleprompt_core::ident::assign_ids;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::{resolve, Item, Program};
use teleprompt_core::Hash;

const SRC: &str = r#"---
timing:
  lead_in_ms: 200
scene:
  mock: { adapter: mock }
---

# Intro

Welcome. {#welcome}

```teleprompt scene=mock policy=concurrent
wait 500ms
```

Second. {#second lead_in=400ms}

<!-- teleprompt: pause 800ms -->
"#;

fn program() -> teleprompt_core::program::Program {
    let mut s = parse_script(SRC).unwrap();
    teleprompt_core::ident::assign_ids(&mut s);
    resolve(
        &s,
        "demo.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .unwrap()
}

fn program_for(src: &str) -> Program {
    let mut parsed = parse_script(src).expect("fixture parses");
    let diags = assign_ids(&mut parsed);
    assert!(
        !diags.iter().any(|d| d.is_error()),
        "fixture must yield unambiguous segment ids"
    );
    resolve(
        &parsed,
        "tour.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .expect("fixture resolves")
}

#[test]
fn chapters_flatten_into_one_ordered_item_list() {
    let p = program();
    let kinds: Vec<&str> = p
        .items
        .iter()
        .map(|i| match i {
            Item::Narration { .. } => "narration",
            Item::Action { .. } => "action",
            Item::Pause { .. } => "pause",
        })
        .collect();
    assert_eq!(kinds, ["narration", "action", "narration", "pause"]);
}

#[test]
fn front_matter_config_reaches_every_item() {
    let p = program();
    let Item::Narration { config, .. } = &p.items[0] else {
        panic!()
    };
    assert_eq!(config.timing.lead_in_ms, 200);
}

#[test]
fn segment_attributes_override_front_matter_for_that_segment_only() {
    let p = program();
    let Item::Narration { config: first, .. } = &p.items[0] else {
        panic!()
    };
    let Item::Narration { config: second, .. } = &p.items[2] else {
        panic!()
    };
    assert_eq!(first.timing.lead_in_ms, 200);
    assert_eq!(second.timing.lead_in_ms, 400);
}

#[test]
fn cli_flags_beat_everything() {
    let mut s = parse_script(SRC).unwrap();
    teleprompt_core::ident::assign_ids(&mut s);
    let cli = PartialConfig::from_yaml("timing:\n  lead_in_ms: 999\n").unwrap();
    let p = resolve(&s, "demo.md", "en", &PartialConfig::default(), &cli).unwrap();
    let Item::Narration { config, .. } = &p.items[2] else {
        panic!()
    };
    assert_eq!(
        config.timing.lead_in_ms, 999,
        "CLI outranks the segment attribute"
    );
}

#[test]
fn source_hash_covers_the_normalised_text_only() {
    let p = program();
    let Item::Narration {
        source_hash, text, ..
    } = &p.items[0]
    else {
        panic!()
    };
    assert_eq!(*source_hash, Hash::of(text.as_bytes()));
}

#[test]
fn block_policy_and_align_default_when_unset() {
    let mut s = parse_script("# A\n\nOne.\n\n```teleprompt scene=mock\nwait 1s\n```\n").unwrap();
    teleprompt_core::ident::assign_ids(&mut s);
    let p = resolve(
        &s,
        "d.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .unwrap();
    let Item::Action { policy, align, .. } = &p.items[1] else {
        panic!()
    };
    assert_eq!(policy, "hold");
    assert_eq!(align, "start");
}

#[test]
fn an_action_block_without_a_scene_is_an_error() {
    let mut s = parse_script("# A\n\nOne.\n\n```teleprompt\nwait 1s\n```\n").unwrap();
    teleprompt_core::ident::assign_ids(&mut s);
    let e = resolve(
        &s,
        "d.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .unwrap_err();
    assert!(e.0[0].message.contains("action block has no `scene`"));
}

#[test]
fn unknown_attribute_keys_surface_as_errors() {
    let mut s = parse_script("# A\n\nOne. {#a polcy=hold}\n").unwrap();
    teleprompt_core::ident::assign_ids(&mut s);
    let e = resolve(
        &s,
        "d.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .unwrap_err();
    assert!(e.0[0].message.contains("unknown attribute key `polcy`"));
}

/// Controller ruling F12: `Item` carries the source span from the AST node it
/// was built from, so Task 11 can point an adapter validation error at the
/// real line instead of a fabricated `line: 0`. This script puts the
/// narration paragraph and the action fence on different lines, so the test
/// would fail if the spans were swapped or both zeroed.
#[test]
fn item_spans_match_their_source_node_not_a_fabricated_line() {
    let src = "# A\n\nFirst line. {#first}\n\n```teleprompt scene=mock\nwait 1s\n```\n";
    let mut s = parse_script(src).unwrap();
    teleprompt_core::ident::assign_ids(&mut s);
    let p = resolve(
        &s,
        "d.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .unwrap();

    let Item::Narration {
        span: narration_span,
        ..
    } = &p.items[0]
    else {
        panic!("expected narration item")
    };
    let Item::Action {
        span: action_span, ..
    } = &p.items[1]
    else {
        panic!("expected action item")
    };

    assert_eq!(narration_span.line, 3, "paragraph sits on line 3");
    assert_eq!(action_span.line, 5, "fence opens on line 5");
    assert_ne!(narration_span.line, action_span.line);
}

#[test]
fn resolve_records_chapters_in_document_order() {
    let src = "\
# Quick start

The first paragraph.

# Provenance

The second paragraph.
";
    let program = program_for(src);

    let chapters: Vec<(&str, &str)> = program
        .chapters
        .iter()
        .map(|c| (c.slug.as_str(), c.title.as_str()))
        .collect();
    assert_eq!(
        chapters,
        vec![("quick-start", "Quick start"), ("provenance", "Provenance")]
    );
}

#[test]
fn every_narration_names_its_owning_chapter() {
    let src = "\
# Quick start

The first paragraph.

# Provenance

The second paragraph.
";
    let program = program_for(src);

    let owners: Vec<&str> = program
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Narration { chapter, .. } => Some(chapter.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(owners, vec!["quick-start", "provenance"]);
}

#[test]
fn a_chapter_with_no_narration_is_still_recorded() {
    let src = "\
# Intro

Only this chapter speaks.

# Silent
";
    let program = program_for(src);

    assert_eq!(
        program.chapters.len(),
        2,
        "resolve reports the script's shape, not just the spoken parts"
    );
    assert_eq!(program.chapters[1].slug, "silent");
}

/// C1. Moving duration prediction off the backend dropped the backend's own
/// `speed <= 0.0` rejection from the offline path: `speed: 0` divided into
/// `+inf`, saturated to `u64::MAX`, and overflowed the scheduler's padding
/// add — a panic on `check`. `speed: -1` was quieter and worse: `check`
/// accepted it and only `dub`'s synthesis refused it, so the validate-only
/// command passed a script the real command would not run.
///
/// The value can arrive from any layer, so the check lives on the merged
/// config — the one the estimator is actually handed.
fn resolve_err(front: &str) -> Vec<String> {
    let src = format!("---\n{front}\n---\n\n# A\n\nOne sentence. {{#a}}\n");
    let mut s = parse_script(&src).expect("fixture parses");
    assign_ids(&mut s);
    resolve(
        &s,
        "d.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .expect_err("a bad voice.speed must not resolve")
    .0
    .iter()
    .map(|d| d.message.clone())
    .collect()
}

#[test]
fn a_zero_voice_speed_is_a_validation_error() {
    let msgs = resolve_err("voice: { speed: 0 }");
    assert!(
        msgs.iter()
            .any(|m| m.contains("speed must be greater than zero")),
        "{msgs:?}"
    );
}

#[test]
fn a_negative_voice_speed_is_a_validation_error() {
    let msgs = resolve_err("voice: { speed: -1 }");
    assert!(
        msgs.iter()
            .any(|m| m.contains("speed must be greater than zero")),
        "{msgs:?}"
    );
}

#[test]
fn a_nan_voice_speed_is_a_validation_error() {
    let msgs = resolve_err("voice: { speed: .nan }");
    assert!(
        msgs.iter()
            .any(|m| m.contains("speed must be a finite number greater than zero")),
        "{msgs:?}"
    );
}

#[test]
fn an_infinite_voice_speed_is_a_validation_error() {
    let msgs = resolve_err("voice: { speed: .inf }");
    assert!(
        msgs.iter()
            .any(|m| m.contains("speed must be a finite number greater than zero")),
        "{msgs:?}"
    );
}

/// A positive speed is not a problem, and the diagnostic must not fire on
/// the default either — a validation that rejects every script is not a
/// validation.
#[test]
fn ordinary_voice_speeds_resolve() {
    for speed in ["0.5", "1.0", "2.0"] {
        let src = format!("---\nvoice: {{ speed: {speed} }}\n---\n\n# A\n\nOne. {{#a}}\n");
        let mut s = parse_script(&src).expect("fixture parses");
        assign_ids(&mut s);
        resolve(
            &s,
            "d.md",
            "en",
            &PartialConfig::default(),
            &PartialConfig::default(),
        )
        .unwrap_or_else(|e| panic!("speed {speed} must resolve, got {:?}", e.0));
    }
}

/// One bad value in front matter is one diagnostic, however many paragraphs
/// resolve against it — and it points at a real segment rather than nowhere.
#[test]
fn one_bad_speed_reports_once_and_names_a_line() {
    let src = "---\nvoice: { speed: 0 }\n---\n\n# A\n\nOne. {#a}\n\nTwo. {#b}\n\nThree. {#c}\n";
    let mut s = parse_script(src).expect("fixture parses");
    assign_ids(&mut s);
    let e = resolve(
        &s,
        "d.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .expect_err("speed 0 must not resolve");
    assert_eq!(e.0.len(), 1, "{:?}", e.0);
    assert!(e.0[0].span.is_some(), "diagnostic must point at a segment");
}

/// A segment attribute is one of the five layers the value can arrive from,
/// and it is the last one before the CLI — so the merged check has to see it
/// even when the script-level config is perfectly fine.
#[test]
fn a_segment_level_speed_override_is_validated_too() {
    let src = "# A\n\nOne. {#a voice.speed=0}\n";
    let mut s = parse_script(src).expect("fixture parses");
    assign_ids(&mut s);
    let e = resolve(
        &s,
        "d.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .expect_err("a segment-level speed 0 must not resolve");
    assert!(
        e.0.iter()
            .any(|d| d.message.contains("speed must be greater than zero")),
        "{:?}",
        e.0
    );
}
