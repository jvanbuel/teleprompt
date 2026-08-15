use teleprompt_core::config::PartialConfig;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::{resolve, Item};
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
