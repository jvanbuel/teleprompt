use teleprompt_core::ast::{Directive, Node};
use teleprompt_core::parse::parse_script;

const BASIC: &str = r#"---
teleprompt: 1
---

# Quickstart

Welcome to Acme. {#welcome}

```teleprompt scene=browser
await page.goto('/');
```

Second paragraph here.
"#;

#[test]
fn extracts_front_matter() {
    let s = parse_script(BASIC).unwrap();
    assert!(s.front_matter.contains("teleprompt: 1"));
}

#[test]
fn heading_becomes_chapter_with_slug() {
    let s = parse_script(BASIC).unwrap();
    assert_eq!(s.chapters.len(), 1);
    assert_eq!(s.chapters[0].title, "Quickstart");
    assert_eq!(s.chapters[0].slug, "quickstart");
}

#[test]
fn paragraphs_become_segments_and_fences_become_action_blocks() {
    let s = parse_script(BASIC).unwrap();
    let kinds: Vec<&str> = s.chapters[0]
        .nodes
        .iter()
        .map(|n| match n {
            Node::Segment(_) => "segment",
            Node::ActionBlock(_) => "action",
            Node::Directive(_) => "directive",
        })
        .collect();
    assert_eq!(kinds, ["segment", "action", "segment"]);
}

#[test]
fn action_block_body_is_verbatim() {
    let s = parse_script(BASIC).unwrap();
    let Node::ActionBlock(b) = &s.chapters[0].nodes[1] else {
        panic!("expected action block")
    };
    assert_eq!(b.body.trim(), "await page.goto('/');");
    assert_eq!(b.info, "scene=browser");
}

#[test]
fn segment_text_excludes_the_attribute_suffix() {
    let s = parse_script(BASIC).unwrap();
    let Node::Segment(seg) = &s.chapters[0].nodes[0] else {
        panic!("expected segment")
    };
    assert_eq!(seg.text, "Welcome to Acme.");
    assert_eq!(seg.raw_attrs, "#welcome");
}

#[test]
fn non_teleprompt_fences_are_ignored() {
    let src = "# C\n\nText.\n\n```rust\nfn main() {}\n```\n";
    let s = parse_script(src).unwrap();
    assert_eq!(s.chapters[0].nodes.len(), 1);
}

#[test]
fn lists_and_tables_are_ignored() {
    let src = "# C\n\nText.\n\n- a\n- b\n\n| x | y |\n|---|---|\n| 1 | 2 |\n";
    let s = parse_script(src).unwrap();
    assert_eq!(s.chapters[0].nodes.len(), 1);
}

#[test]
fn html_comment_paragraph_is_a_directive() {
    let src = "# C\n\n<!-- teleprompt: pause 800ms -->\n";
    let s = parse_script(src).unwrap();
    assert!(matches!(
        s.chapters[0].nodes[0],
        Node::Directive(Directive::Pause(800))
    ));
}

#[test]
fn inline_markup_is_normalised_for_speech() {
    let src = "# C\n\nUse *the* `--watch` [flag](https://x.dev).\n";
    let s = parse_script(src).unwrap();
    let Node::Segment(seg) = &s.chapters[0].nodes[0] else {
        panic!("expected segment")
    };
    assert_eq!(seg.text, "Use the --watch flag.");
}

#[test]
fn content_before_any_heading_is_an_error() {
    let src = "Orphan paragraph.\n\n# C\n\nText.\n";
    let err = parse_script(src).unwrap_err();
    assert!(err.0[0].message.contains("before the first heading"));
}
