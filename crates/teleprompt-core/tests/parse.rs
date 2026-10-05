use teleprompt_core::ast::{Directive, Node};
use teleprompt_core::parse::parse_script;
use teleprompt_core::DurationMs;

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
fn paragraphs_become_lines_and_fences_become_action_blocks() {
    let s = parse_script(BASIC).unwrap();
    let kinds: Vec<&str> = s.chapters[0]
        .nodes
        .iter()
        .map(|n| match n {
            Node::Line(_) => "line",
            Node::ActionBlock(_) => "action",
            Node::Directive(_) => "directive",
        })
        .collect();
    assert_eq!(kinds, ["line", "action", "line"]);
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
fn line_text_excludes_the_attribute_suffix() {
    let s = parse_script(BASIC).unwrap();
    let Node::Line(seg) = &s.chapters[0].nodes[0] else {
        panic!("expected line")
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
        Node::Directive(Directive::Pause(ms)) if ms == DurationMs::millis(800)
    ));
}

#[test]
fn inline_markup_is_normalised_for_speech() {
    let src = "# C\n\nUse *the* `--watch` [flag](https://x.dev).\n";
    let s = parse_script(src).unwrap();
    let Node::Line(seg) = &s.chapters[0].nodes[0] else {
        panic!("expected line")
    };
    assert_eq!(seg.text, "Use the --watch flag.");
}

#[test]
fn content_before_any_heading_is_an_error() {
    let src = "Orphan paragraph.\n\n# C\n\nText.\n";
    let err = parse_script(src).unwrap_err();
    assert!(err.0[0].message.contains("before the first heading"));
}

#[test]
fn a_soft_wrapped_paragraph_joins_lines_with_a_single_space() {
    let src = "# C\n\nrolling back takes the same single command with one\nextra flag.\n";
    let s = parse_script(src).unwrap();
    let Node::Line(seg) = &s.chapters[0].nodes[0] else {
        panic!("expected line")
    };
    assert_eq!(
        seg.text,
        "rolling back takes the same single command with one extra flag."
    );
}

#[test]
fn a_paragraph_wrapped_across_three_lines_has_no_doubled_spaces() {
    let src = "# C\n\none\ntwo\nthree\n";
    let s = parse_script(src).unwrap();
    let Node::Line(seg) = &s.chapters[0].nodes[0] else {
        panic!("expected line")
    };
    assert_eq!(seg.text, "one two three");
    assert!(!seg.text.contains("  "));
}

#[test]
fn a_hard_break_via_trailing_spaces_is_a_single_space() {
    let src = "# C\n\none  \ntwo\n";
    let s = parse_script(src).unwrap();
    let Node::Line(seg) = &s.chapters[0].nodes[0] else {
        panic!("expected line")
    };
    assert_eq!(seg.text, "one two");
}

#[test]
fn a_hard_break_via_backslash_is_a_single_space() {
    let src = "# C\n\none\\\ntwo\n";
    let s = parse_script(src).unwrap();
    let Node::Line(seg) = &s.chapters[0].nodes[0] else {
        panic!("expected line")
    };
    assert_eq!(seg.text, "one two");
}

#[test]
fn unterminated_front_matter_is_an_error() {
    let src = "---\nteleprompt: 1\n\n# C\n\nText.\n";
    let err = parse_script(src).unwrap_err();
    assert!(
        err.0[0].message.contains("no closing `---` line was found"),
        "message was: {}",
        err.0[0].message
    );
    let help = err.0[0].help.as_deref().unwrap_or("");
    assert!(
        help.contains("horizontal rule"),
        "help should also name the horizontal-rule reading so an author who \
         meant a divider knows what to do; help was: {help}"
    );
}

#[test]
fn a_malformed_pause_value_is_an_error_mentioning_the_bad_value() {
    let src = "# C\n\n<!-- teleprompt: pause 80oms -->\n";
    let err = parse_script(src).unwrap_err();
    assert!(
        err.0[0].message.contains("80oms"),
        "message was: {}",
        err.0[0].message
    );
}

#[test]
fn an_unknown_teleprompt_directive_is_an_error_naming_it() {
    let src = "# C\n\n<!-- teleprompt: frobnicate -->\n";
    let err = parse_script(src).unwrap_err();
    assert!(
        err.0[0].message.contains("frobnicate"),
        "message was: {}",
        err.0[0].message
    );
}

#[test]
fn an_ordinary_html_comment_is_silently_ignored() {
    let src = "# C\n\n<!-- just a note -->\n\nText.\n";
    let s = parse_script(src).unwrap();
    assert_eq!(s.chapters[0].nodes.len(), 1);
}

#[test]
fn two_chapter_config_blocks_back_to_back_is_an_error_naming_the_chapter() {
    let src = "# Deploying\n\n```yaml teleprompt\ntiming:\n  lead_in_ms: 100\n```\n\n\
               ```yaml teleprompt\ntiming:\n  lead_in_ms: 200\n```\n\nText.\n";
    let err = parse_script(src).unwrap_err();
    assert!(
        err.0[0].message.contains("deploying"),
        "message was: {}",
        err.0[0].message
    );
}

#[test]
fn a_wrapped_paragraph_matches_the_same_prose_on_one_line() {
    let wrapped = "# C\n\nrolling back takes the same single command with one\nextra flag.\n";
    let one_line = "# C\n\nrolling back takes the same single command with one extra flag.\n";
    let a = parse_script(wrapped).unwrap();
    let b = parse_script(one_line).unwrap();
    let Node::Line(sa) = &a.chapters[0].nodes[0] else {
        panic!("expected line")
    };
    let Node::Line(sb) = &b.chapters[0].nodes[0] else {
        panic!("expected line")
    };
    assert_eq!(sa.text, sb.text);
}

// ---------------------------------------------------------------------------
// `split_attr_suffix` ran `ends_with('}')` + `rfind('{')` over the *normalised*
// paragraph text, after backticks were stripped, so it could not tell an inline
// code shot from an attribute block. teleprompt's own scripts are documentation
// full of config snippets, so this rejected valid prose:
//
//   $ cat scripts/j1.md   # The config block looks like `{ fps: 30 }`
//   error: expected `key=value`, found `fps:`
// ---------------------------------------------------------------------------

fn only_line(src: &str) -> teleprompt_core::ast::Line {
    let s = parse_script(src).expect("script must parse");
    match &s.chapters[0].nodes[0] {
        Node::Line(seg) => seg.clone(),
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn a_paragraph_ending_in_an_inline_code_shot_with_braces_is_narration() {
    let seg = only_line("# B\n\nThe config block looks like `{ fps: 30 }`\n");
    assert_eq!(
        seg.text, "The config block looks like { fps: 30 }",
        "the code shot's text must survive intact into the narration"
    );
    assert_eq!(seg.raw_attrs, "", "no attribute suffix was written here");
}

#[test]
fn a_real_id_suffix_still_parses() {
    let seg = only_line("# B\n\nGive it a name and you are done. {#done}\n");
    assert_eq!(seg.text, "Give it a name and you are done.");
    assert_eq!(seg.raw_attrs, "#done");
}

#[test]
fn a_real_key_value_suffix_still_parses() {
    let seg = only_line("# B\n\nGive it a name. {#done voice.backend=kokoro}\n");
    assert_eq!(seg.text, "Give it a name.");
    assert_eq!(seg.raw_attrs, "#done voice.backend=kokoro");
}

/// A code shot that is *not* at the paragraph's end must not suppress a real
/// suffix that follows it.
#[test]
fn a_code_shot_before_a_real_suffix_does_not_suppress_it() {
    let seg = only_line("# B\n\nThe block looks like `{ fps: 30 }` in practice. {#mixed}\n");
    assert_eq!(seg.text, "The block looks like { fps: 30 } in practice.");
    assert_eq!(seg.raw_attrs, "#mixed");
}

/// The reason this fix tracks the code shot rather than pattern-matching the
/// suffix's shape: a genuinely malformed attribute block must stay an error.
/// A shape test would reclassify this as prose and silently drop it.
#[test]
fn a_malformed_attribute_suffix_is_still_reported_not_silently_prose() {
    let e = parse_script("# B\n\nGive it a name. {polcy hold}\n");
    match e {
        Ok(s) => {
            let Node::Line(seg) = &s.chapters[0].nodes[0] else {
                panic!("expected line")
            };
            assert_eq!(
                seg.raw_attrs, "polcy hold",
                "a bare trailing brace block is still read as attributes, so \
                 `parse_attrs` can report it"
            );
        }
        Err(d) => panic!("parse itself must not fail here: {:?}", d.0),
    }
}

/// Something that starts partway through a line is reported on that line,
/// at its own column (#23). Counting `lines()` up to it counted the
/// unfinished line as well, one too many.
#[test]
fn a_mid_line_directive_is_reported_where_it_is() {
    let src =
        "---\nteleprompt: 1\n---\n\n# Intro\n\nHello there <!-- teleprompt: bogus --> friend.\n";
    let err = parse_script(src).unwrap_err();
    let span = err.0[0].span.expect("the diagnostic points somewhere");
    assert_eq!((span.line, span.column), (7, 13));
}

/// A directive inside a heading came before that heading's chapter existed,
/// so it joined the previous chapter (#23). It is rejected instead, at the
/// heading, saying where directives go.
#[test]
fn a_directive_inside_a_heading_is_rejected() {
    let src = "---\nteleprompt: 1\n---\n\n# One\n\nFirst. {#a}\n\n# Two <!-- teleprompt: pause 500ms -->\n\nSecond. {#b}\n";
    let err = parse_script(src).unwrap_err();
    let d = &err.0[0];
    assert!(d.message.contains("own line"), "{}", d.message);
    assert_eq!(d.span.expect("points at the heading").line, 9);
}

/// A pause is bounded like any other duration, so an absurd one is a
/// diagnostic here rather than a sum near `u64::MAX` in the scheduler (#22).
#[test]
fn a_pause_longer_than_a_day_is_rejected() {
    let src = "# One\n\nFirst. {#a}\n\n<!-- teleprompt: pause 18446744073709551615ms -->\n";
    let err = parse_script(src).unwrap_err();
    assert!(
        err.0[0].message.contains("longer than a day"),
        "{}",
        err.0[0].message
    );
}

/// Front matter with nothing in it is still front matter, closed on the
/// next line; the lines after it keep their numbers.
#[test]
fn an_empty_front_matter_closes() {
    let s = parse_script("---\n---\n\n# A\n\nOne. {#one}\n").unwrap();
    assert_eq!(s.front_matter, "");
    assert_eq!(s.chapters[0].span.line, 4);
}
