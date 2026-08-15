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
