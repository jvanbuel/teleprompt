use teleprompt_script::ast::Node;
use teleprompt_script::ident::{check_ids, IdOrigin};
use teleprompt_script::parse::parse_script;

fn ids(src: &str) -> Vec<String> {
    let s = parse_script(src).unwrap();
    let diags = check_ids(&s);
    assert!(
        !diags.iter().any(|d| d.is_error()),
        "unexpected errors: {diags:?}"
    );
    s.chapters
        .iter()
        .flat_map(|c| c.nodes.iter())
        .filter_map(|n| match n {
            Node::Line(seg) => Some(seg.id.to_string()),
            Node::ActionBlock(b) => Some(b.id.to_string()),
            Node::Directive(_) => None,
        })
        .collect()
}

#[test]
fn derived_ids_number_lines_within_a_chapter() {
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
fn action_block_derives_from_the_preceding_line() {
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
    let s = parse_script(src).unwrap();
    let origins: Vec<IdOrigin> = s.chapters[0]
        .nodes
        .iter()
        .filter_map(|n| match n {
            Node::Line(seg) => Some(seg.id_origin),
            _ => None,
        })
        .collect();
    assert_eq!(origins, [IdOrigin::Explicit, IdOrigin::Derived]);
}

#[test]
fn duplicate_explicit_ids_are_an_error() {
    let src = "# A\n\nOne. {#dup}\n\nTwo. {#dup}\n";
    let s = parse_script(src).unwrap();
    let diags = check_ids(&s);
    assert!(diags
        .iter()
        .any(|d| d.is_error() && d.message.contains("duplicate line id `dup`")));
}

#[test]
fn explicit_id_colliding_with_a_derived_id_is_an_error() {
    let src = "# A\n\nOne.\n\nTwo. {#a-1}\n";
    let s = parse_script(src).unwrap();
    let diags = check_ids(&s);
    assert!(diags
        .iter()
        .any(|d| d.is_error() && d.message.contains("duplicate line id `a-1`")));
}

#[test]
fn adjacent_action_blocks_after_one_line_get_distinct_derived_ids() {
    let src = "# A\n\nOne.\n\n```teleprompt scene=mock\nwait 100ms\n```\n\n```teleprompt scene=mock\nwait 200ms\n```\n";
    assert_eq!(ids(src), ["a-1", "a-1-a", "a-1-a2"]);
}

#[test]
fn action_block_counter_resets_after_a_new_line() {
    let src = "# A\n\nOne.\n\n```teleprompt scene=mock\nwait 100ms\n```\n\nTwo.\n\n```teleprompt scene=mock\nwait 200ms\n```\n";
    assert_eq!(ids(src), ["a-1", "a-1-a", "a-2", "a-2-a"]);
}

#[test]
fn block_id_colliding_with_an_explicit_line_id_is_an_error() {
    let src = "# A\n\nOne.\n\n```teleprompt scene=mock\nwait 100ms\n```\n\nTwo. {#a-1-a}\n";
    let s = parse_script(src).unwrap();
    let diags = check_ids(&s);
    assert!(diags
        .iter()
        .any(|d| d.is_error() && d.message.contains("duplicate line id `a-1-a`")));
}

fn diags_for(src: &str) -> Vec<teleprompt_core::Diagnostic> {
    let s = parse_script(src).unwrap();
    check_ids(&s)
}

/// The reproduced hazard: an id becomes `audio/<id>.wav`, so this one
/// writes outside `--out` and publishes an escaping path in the manifest.
/// It has to fail at `check` time, in `check_ids`, not be sanitised at the
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
    // `{#has space}` never reaches `check_ids` as one id.
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

/// End to end for the relaxed rule: a non-ASCII heading must survive the
/// whole of `check`, not merely `check_ids`, and must produce the audio
/// path a consumer will resolve.
#[test]
fn a_non_ascii_id_is_a_usable_audio_path() {
    let id = &ids("# Café\n\nOne.\n")[0];
    let path = format!("audio/{id}.wav");
    assert_eq!(path, "audio/café-1.wav");
    assert!(
        !path.contains("..") && !path[6..].contains('/'),
        "still one path line, still inside `audio/`: {path}"
    );
}

/// Not a tab or a newline: those are whitespace, and the parser takes an
/// id only up to the first whitespace, so they never arrive here as part
/// of one. DEL and the C0 controls do arrive, and they are the half of the
/// hazard that is about the filesystem rather than about traversal.
#[test]
fn a_control_character_in_an_id_is_an_error() {
    for src in [
        "# A\n\nOne. {#del\u{7f}here}\n",
        "# A\n\nOne. {#bell\u{7}}\n",
    ] {
        let diags = diags_for(src);
        assert!(
            diags
                .iter()
                .any(|d| d.is_error() && d.message.contains("control character")),
            "{src:?} should be rejected, got {diags:?}"
        );
    }
}

/// The rule covers derived ids too, not just explicit ones: a derived id
/// becomes exactly the same file name. But the hazard is path traversal
/// and control characters, **not** non-ASCII letters — teleprompt is a
/// localization-first tool, and a heading in the languages it exists to
/// dub must not be an error. `café-1.wav` is a perfectly good file name.
#[test]
fn a_non_ascii_heading_yields_a_usable_derived_id() {
    assert_eq!(ids("# Café\n\nOne.\n"), ["café-1"]);
    assert_eq!(ids("# Развёртывание\n\nOne.\n"), ["развёртывание-1"]);
    assert_eq!(ids("# 配置\n\nOne.\n"), ["配置-1"]);
}

#[test]
fn a_non_ascii_explicit_id_is_accepted() {
    assert_eq!(ids("# A\n\nOne. {#präsentation}\n"), ["präsentation"]);
}

#[test]
fn empty_explicit_id_is_an_error() {
    let diags = parse_script("# A\n\nOne. {#}\n").unwrap_err().0;
    assert!(diags
        .iter()
        .any(|d| d.is_error() && d.message.contains("line id cannot be empty")));
}

/// A block's own `id=` names it, as the attribute table says.
#[test]
fn a_block_s_own_id_is_its_id() {
    let src = "# A\n\nOne.\n\n```teleprompt scene=mock id=deploy\nwait 100ms\n```\n";
    assert_eq!(ids(src), ["a-1", "deploy"]);
}

/// Pinning one block's id renames no other: a block is counted whether its
/// id was derived or given.
#[test]
fn pinning_a_block_s_id_renames_no_other() {
    let src = "# A\n\nOne.\n\n```teleprompt scene=mock id=first\nwait 100ms\n```\n\n\
               ```teleprompt scene=mock\nwait 200ms\n```\n";
    assert_eq!(ids(src), ["a-1", "first", "a-1-a2"]);
}

/// Nothing compiles a script whose ids were not checked: `resolve` checks
/// them itself.
#[test]
fn resolve_refuses_a_duplicate_id() {
    let s = parse_script("# A\n\nOne. {#dup}\n\nTwo. {#dup}\n").unwrap();
    let err = teleprompt_script::program::resolve(
        &s,
        "t.md",
        "en",
        &teleprompt_core::config::PartialConfig::default(),
        &teleprompt_core::config::PartialConfig::default(),
    )
    .unwrap_err();
    assert!(
        err.0
            .iter()
            .any(|d| d.message.contains("duplicate line id `dup`")),
        "{err:?}"
    );
}
