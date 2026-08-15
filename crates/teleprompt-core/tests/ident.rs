use teleprompt_core::ast::Node;
use teleprompt_core::ident::{assign_ids, IdOrigin};
use teleprompt_core::parse::parse_script;

fn ids(src: &str) -> Vec<String> {
    let mut s = parse_script(src).unwrap();
    let diags = assign_ids(&mut s);
    assert!(
        !diags.iter().any(|d| d.is_error()),
        "unexpected errors: {diags:?}"
    );
    s.chapters
        .iter()
        .flat_map(|c| c.nodes.iter())
        .filter_map(|n| match n {
            Node::Segment(seg) => seg.id.clone(),
            Node::ActionBlock(b) => b.id.clone(),
            Node::Directive(_) => None,
        })
        .collect()
}

#[test]
fn derived_ids_number_segments_within_a_chapter() {
    let src = "# Quick Start\n\nOne.\n\nTwo.\n";
    assert_eq!(ids(src), ["quick-start-1", "quick-start-2"]);
}

#[test]
fn numbering_restarts_per_chapter() {
    let src = "# A\n\nOne.\n\n# B\n\nTwo.\n";
    assert_eq!(ids(src), ["a-1", "b-1"]);
}

#[test]
fn explicit_id_wins_and_does_not_consume_an_ordinal() {
    let src = "# A\n\nOne. {#welcome}\n\nTwo.\n";
    assert_eq!(ids(src), ["welcome", "a-2"]);
}

#[test]
fn action_block_derives_from_the_preceding_segment() {
    let src = "# A\n\nOne.\n\n```teleprompt scene=mock\nwait 100ms\n```\n";
    assert_eq!(ids(src), ["a-1", "a-1-a"]);
}

#[test]
fn leading_action_block_derives_from_the_chapter() {
    let src = "# A\n\n```teleprompt scene=mock\nwait 100ms\n```\n\nOne.\n";
    assert_eq!(ids(src), ["a-b1", "a-1"]);
}

#[test]
fn id_origin_is_recorded() {
    let src = "# A\n\nOne. {#welcome}\n\nTwo.\n";
    let mut s = parse_script(src).unwrap();
    assign_ids(&mut s);
    let origins: Vec<IdOrigin> = s.chapters[0]
        .nodes
        .iter()
        .filter_map(|n| match n {
            Node::Segment(seg) => Some(seg.id_origin),
            _ => None,
        })
        .collect();
    assert_eq!(origins, [IdOrigin::Explicit, IdOrigin::Derived]);
}

#[test]
fn duplicate_explicit_ids_are_an_error() {
    let src = "# A\n\nOne. {#dup}\n\nTwo. {#dup}\n";
    let mut s = parse_script(src).unwrap();
    let diags = assign_ids(&mut s);
    assert!(diags
        .iter()
        .any(|d| d.is_error() && d.message.contains("duplicate segment id `dup`")));
}

#[test]
fn explicit_id_colliding_with_a_derived_id_is_an_error() {
    let src = "# A\n\nOne.\n\nTwo. {#a-1}\n";
    let mut s = parse_script(src).unwrap();
    let diags = assign_ids(&mut s);
    assert!(diags
        .iter()
        .any(|d| d.is_error() && d.message.contains("duplicate segment id `a-1`")));
}

#[test]
fn adjacent_action_blocks_after_one_segment_get_distinct_derived_ids() {
    let src = "# A\n\nOne.\n\n```teleprompt scene=mock\nwait 100ms\n```\n\n```teleprompt scene=mock\nwait 200ms\n```\n";
    assert_eq!(ids(src), ["a-1", "a-1-a", "a-1-a2"]);
}

#[test]
fn action_block_counter_resets_after_a_new_segment() {
    let src = "# A\n\nOne.\n\n```teleprompt scene=mock\nwait 100ms\n```\n\nTwo.\n\n```teleprompt scene=mock\nwait 200ms\n```\n";
    assert_eq!(ids(src), ["a-1", "a-1-a", "a-2", "a-2-a"]);
}

#[test]
fn block_id_colliding_with_an_explicit_segment_id_is_an_error() {
    let src = "# A\n\nOne.\n\n```teleprompt scene=mock\nwait 100ms\n```\n\nTwo. {#a-1-a}\n";
    let mut s = parse_script(src).unwrap();
    let diags = assign_ids(&mut s);
    assert!(diags
        .iter()
        .any(|d| d.is_error() && d.message.contains("duplicate segment id `a-1-a`")));
}

fn diags_for(src: &str) -> Vec<teleprompt_core::Diagnostic> {
    let mut s = parse_script(src).unwrap();
    assign_ids(&mut s)
}

/// The reproduced hazard: an id becomes `audio/<id>.wav`, so this one
/// writes outside `--out` and publishes an escaping path in the manifest.
/// It has to fail at `check` time, in `assign_ids`, not be sanitised at the
/// write site — every command must reject the same ids.
#[test]
fn an_id_that_escapes_its_directory_is_an_error() {
    let diags = diags_for("# A\n\nOne. {#../../../pwned}\n");
    assert!(
        diags
            .iter()
            .any(|d| d.is_error() && d.message.contains("../../../pwned")),
        "{diags:?}"
    );
}

#[test]
fn a_path_separator_in_an_id_is_an_error() {
    for src in [
        "# A\n\nOne. {#sub/dir}\n",
        "# A\n\nOne. {#sub\\dir}\n",
        "# A\n\nOne. {#/absolute}\n",
    ] {
        let diags = diags_for(src);
        assert!(
            diags.iter().any(|d| d.is_error()),
            "{src:?} should be rejected, got {diags:?}"
        );
    }
}

#[test]
fn a_dot_only_id_is_an_error() {
    for src in ["# A\n\nOne. {#.}\n", "# A\n\nOne. {#..}\n"] {
        let diags = diags_for(src);
        assert!(
            diags.iter().any(|d| d.is_error()),
            "{src:?} should be rejected, got {diags:?}"
        );
    }
}

#[test]
fn a_leading_dot_is_an_error_even_without_a_separator() {
    // `.hidden` cannot traverse anywhere, but it is a dotfile on every
    // platform teleprompt runs on and would be invisible in the output
    // directory a consumer is told to serve.
    let diags = diags_for("# A\n\nOne. {#.hidden}\n");
    assert!(diags.iter().any(|d| d.is_error()), "{diags:?}");
}

#[test]
fn a_character_outside_the_allowed_set_is_an_error() {
    // Not a space: the parser takes the id up to the first whitespace, so
    // `{#has space}` never reaches `assign_ids` as one id.
    for src in [
        "# A\n\nOne. {#has%25}\n",
        "# A\n\nOne. {#has?query}\n",
        "# A\n\nOne. {#has:colon}\n",
    ] {
        let diags = diags_for(src);
        assert!(
            diags.iter().any(|d| d.is_error()),
            "{src:?} should be rejected, got {diags:?}"
        );
    }
}

#[test]
fn the_allowed_set_is_actually_allowed() {
    // Regression guard on the check itself: `.` mid-id, `_`, `-`, and
    // digits are all fine, and rejecting them would break existing scripts
    // and every derived id.
    assert_eq!(
        ids("# A\n\nOne. {#v1.2_final-b}\n"),
        ["v1.2_final-b"],
        "the check must not be stricter than it claims"
    );
}

/// The rule covers derived ids too, not just explicit ones: a derived id
/// becomes exactly the same file name, and the diagnostic tells the author
/// to pin one by hand.
#[test]
fn a_derived_id_from_a_non_ascii_heading_is_an_error() {
    let diags = diags_for("# Café\n\nOne.\n");
    assert!(
        diags
            .iter()
            .any(|d| d.is_error() && d.message.contains("café-1")),
        "{diags:?}"
    );
    assert!(diags
        .iter()
        .any(|d| d.help.as_deref().unwrap_or_default().contains("{#id}")));
}

#[test]
fn empty_explicit_id_is_an_error() {
    let src = "# A\n\nOne. {#}\n";
    let mut s = parse_script(src).unwrap();
    let diags = assign_ids(&mut s);
    assert!(diags
        .iter()
        .any(|d| d.is_error() && d.message.contains("segment id cannot be empty")));
    let Node::Segment(seg) = &s.chapters[0].nodes[0] else {
        panic!("expected segment")
    };
    assert_eq!(seg.id.as_deref(), Some("a-1"));
    assert_eq!(seg.id_origin, IdOrigin::Derived);
}
