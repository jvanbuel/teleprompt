use std::collections::HashSet;

use crate::ast::{Node, Script};
use crate::{Diagnostic, SourceSpan};

pub use crate::ast::IdOrigin;

/// What an id is allowed to contain.
///
/// An id becomes a file name (`audio/<id>.wav`) that a manifest consumer
/// resolves, so `{#../../pwned}` would write outside `--out`. The check is
/// on the raw string, before anything is joined, for the reason `include=`
/// paths are checked that way, and it runs in `check_ids`, which `resolve` runs, so every
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

/// Every id in `script` checked: each can become a file name, and no two
/// may be the same. The parser gave every line and block one; this is what
/// may be wrong with them. [`crate::program::resolve`] runs it, so nothing
/// compiles a script whose ids it has not checked.
pub fn check_ids(script: &Script) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for node in script.chapters.iter().flat_map(|c| &c.nodes) {
        match node {
            Node::Line(line) => record_id(&mut seen, &line.id, line.span, &mut diags),
            Node::ActionBlock(block) => record_id(&mut seen, &block.id, block.span, &mut diags),
            Node::Directive(_) => {}
        }
    }
    diags
}

/// Validates and records one id. Every id reaches here — derived and
/// explicit, line and action block — because every one of them can end
/// up as a file name, so the check must not be scoped to the explicit ones.
fn record_id<'a>(
    seen: &mut HashSet<&'a str>,
    id: &'a str,
    span: SourceSpan,
    diags: &mut Vec<Diagnostic>,
) {
    if let Some(msg) = id_error(id) {
        diags.push(Diagnostic::error(msg).at(span).with_help(ID_HELP));
    }
    if !seen.insert(id) {
        diags.push(
            Diagnostic::error(format!("duplicate line id `{id}`"))
                .at(span)
                .with_help("give one of them an explicit unique `{#id}`"),
        );
    }
}
