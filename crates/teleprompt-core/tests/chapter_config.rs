use teleprompt_core::config::PartialConfig;
use teleprompt_core::ident::assign_ids;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::{resolve, Element};

const SRC: &str = r#"---
timing:
  lead_in_ms: 100
---

# Fast

One.

# Slow

```yaml teleprompt
timing:
  lead_in_ms: 900
```

Two.
"#;

fn elements() -> Vec<Element> {
    let mut s = parse_script(SRC).unwrap();
    assign_ids(&mut s);
    resolve(
        &s,
        "d.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .unwrap()
    .elements
}

#[test]
fn chapter_front_matter_overrides_script_front_matter() {
    let it = elements();
    let Element::Narration { config: fast, .. } = &it[0] else {
        panic!()
    };
    let Element::Narration { config: slow, .. } = &it[1] else {
        panic!()
    };
    assert_eq!(fast.timing.lead_in_ms, 100);
    assert_eq!(slow.timing.lead_in_ms, 900);
}

#[test]
fn a_chapter_config_block_is_not_narration() {
    assert_eq!(elements().len(), 2, "the yaml block must not become a line");
}

#[test]
fn chapter_front_matter_is_optional() {
    let mut s = parse_script("# A\n\nOne.\n").unwrap();
    assign_ids(&mut s);
    assert!(s.chapters[0].front_matter.is_empty());
}

#[test]
fn chapter_config_after_other_content_is_a_diagnostic() {
    let src = "# A\n\nOne.\n\n```yaml teleprompt\ntiming:\n  lead_in_ms: 900\n```\n";
    let err = parse_script(src).unwrap_err();
    assert!(
        err.0[0]
            .message
            .contains("must come directly after the heading"),
        "message was: {}",
        err.0[0].message
    );
}

#[test]
fn chapter_config_before_any_heading_is_a_diagnostic() {
    let src = "```yaml teleprompt\ntiming:\n  lead_in_ms: 900\n```\n\n# A\n\nOne.\n";
    let err = parse_script(src).unwrap_err();
    assert!(
        err.0[0].message.contains("before the first heading"),
        "message was: {}",
        err.0[0].message
    );
}

#[test]
fn malformed_chapter_front_matter_is_a_diagnostic() {
    let src = "# A\n\n```yaml teleprompt\ntiming: [nope\n```\n\nOne.\n";
    let mut s = parse_script(src).unwrap();
    assign_ids(&mut s);
    let e = resolve(
        &s,
        "d.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .unwrap_err();
    assert!(e.0[0].message.contains("chapter `a` front matter"));
}
