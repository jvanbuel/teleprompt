use std::collections::HashSet;

use crate::ast::{Node, Script};
use crate::{Diagnostic, SourceSpan};

pub use crate::ast::IdOrigin;

/// What an id is allowed to contain.
///
/// An id becomes a file name (`audio/<id>.wav`) that a manifest consumer
/// resolves, so `{#../../pwned}` would write outside `--out`. The check is
/// on the raw string, before anything is joined, for the reason `include=`
/// paths are checked that way, and it runs in `assign_ids` so every
/// command rejects the same ids. Rejecting rather than rewriting keeps an
/// id an author pinned by hand.
///
/// The hazard is paths, not alphabets: the permitted set is Unicode-aware
/// [`char::is_alphanumeric`] plus `-`, `_` and `.`, and control characters
/// get their own branch so the diagnostic can name them.
const ID_HELP: &str = "ids become file names and URL path lines: use letters \
                       (in any script), digits, `-`, `_`, and `.`, and do not \
                       start with `.`; a derived id comes from the chapter \
                       heading, so pin one with `{#id}` when the heading cannot \
                       supply it";

fn id_error(id: &str) -> Option<String> {
    if id == "." || id == ".." {
        return Some(format!("line id `{id}` is a path component"));
    }
    if id.starts_with('.') {
        return Some(format!("line id `{id}` starts with `.`"));
    }
    if id.contains('/') || id.contains('\\') {
        return Some(format!("line id `{id}` contains a path separator"));
    }
    if id.chars().any(char::is_control) {
        return Some(format!("line id `{id}` contains a control character"));
    }
    if let Some(c) = id
        .chars()
        .find(|c| !(c.is_alphanumeric() || *c == '-' || *c == '_' || *c == '.'))
    {
        return Some(format!("line id `{id}` contains `{c}`"));
    }
    None
}

pub fn assign_ids(script: &mut Script) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for chapter in &mut script.chapters {
        let slug = chapter.slug.clone();
        let mut seg_n = 0usize;
        let mut block_n = 0usize;
        let mut seg_block_n = 0usize;
        let mut last_segment: Option<String> = None;

        for node in &mut chapter.nodes {
            match node {
                Node::Line(seg) => {
                    seg_n += 1;
                    seg_block_n = 0;
                    let (id, origin) = match &seg.id {
                        Some(explicit) if explicit.is_empty() => {
                            diags.push(
                                Diagnostic::error("line id cannot be empty")
                                    .at(seg.span)
                                    .with_help("remove the empty `{#}` or give it a non-empty id"),
                            );
                            (format!("{slug}-{seg_n}"), IdOrigin::Derived)
                        }
                        Some(explicit) => (explicit.clone(), IdOrigin::Explicit),
                        None => (format!("{slug}-{seg_n}"), IdOrigin::Derived),
                    };
                    record_id(&mut seen, &id, seg.span, &mut diags);
                    last_segment = Some(id.clone());
                    seg.id = Some(id);
                    seg.id_origin = origin;
                }
                Node::ActionBlock(block) => {
                    if block.id.is_none() {
                        block.id = Some(match &last_segment {
                            Some(seg) => {
                                seg_block_n += 1;
                                if seg_block_n == 1 {
                                    format!("{seg}-a")
                                } else {
                                    format!("{seg}-a{seg_block_n}")
                                }
                            }
                            None => {
                                block_n += 1;
                                format!("{slug}-b{block_n}")
                            }
                        });
                    }
                    let id = block.id.clone().expect("block id was just assigned");
                    record_id(&mut seen, &id, block.span, &mut diags);
                }
                Node::Directive(_) => {}
            }
        }
    }

    diags
}

/// Validates and records one id. Every id reaches here — derived and
/// explicit, line and action block — because every one of them can end
/// up as a file name, so the check must not be scoped to the explicit ones.
fn record_id(seen: &mut HashSet<String>, id: &str, span: SourceSpan, diags: &mut Vec<Diagnostic>) {
    if let Some(msg) = id_error(id) {
        diags.push(Diagnostic::error(msg).at(span).with_help(ID_HELP));
    }
    if !seen.insert(id.to_string()) {
        diags.push(
            Diagnostic::error(format!("duplicate line id `{id}`"))
                .at(span)
                .with_help("give one of them an explicit unique `{#id}`"),
        );
    }
}
