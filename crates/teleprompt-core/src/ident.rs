use std::collections::HashSet;

use crate::ast::{Node, Script};
use crate::{Diagnostic, SourceSpan};

pub use crate::ast::IdOrigin;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SegmentId(pub String);

impl SegmentId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(pub String);

impl BlockId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What an id is allowed to contain.
///
/// An id is not merely a join key: `teleprompt dub` turns it into a file
/// name (`audio/<id>.wav`) and publishes that string as a path inside the
/// narration manifest, which a third-party consumer resolves relative to
/// the manifest. So an explicit `{#../../../pwned}` would write outside
/// `--out` and hand a consumer an escaping path.
///
/// `teleprompt-compile`'s `include=` handling hardens the identical hazard
/// and its reasoning applies verbatim here: joining first and checking
/// afterwards never works, because `..` is just another component and the
/// joined path's prefix is always the root's. The check has to be on the
/// raw string, before anything is built from it.
///
/// It lives in `assign_ids` — the one place every id passes through — so
/// `check`, `plan`, `diff`, and `dub` all reject the same ids, and an
/// author hears about it at `check` time rather than when a file lands in
/// the wrong directory. Rejecting is deliberate: silently rewriting an id
/// would change the join key an author pinned by hand, and break every
/// consumer that already referenced it.
///
/// The permitted set is the intersection of what a file name and a URL
/// path segment both tolerate without escaping: ASCII alphanumerics, `-`,
/// `_`, and `.`.
const ID_HELP: &str = "ids become file names and URL path segments: use ASCII \
                       letters, digits, `-`, `_`, and `.`, and do not start with \
                       `.`; a derived id comes from the chapter heading, so pin \
                       one with `{#id}` when the heading cannot supply it";

fn id_error(id: &str) -> Option<String> {
    if id == "." || id == ".." {
        return Some(format!("segment id `{id}` is a path component"));
    }
    if id.starts_with('.') {
        return Some(format!("segment id `{id}` starts with `.`"));
    }
    if id.contains('/') || id.contains('\\') {
        return Some(format!("segment id `{id}` contains a path separator"));
    }
    if let Some(c) = id
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.'))
    {
        return Some(format!("segment id `{id}` contains `{c}`"));
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
                Node::Segment(seg) => {
                    seg_n += 1;
                    seg_block_n = 0;
                    let (id, origin) = match &seg.id {
                        Some(explicit) if explicit.is_empty() => {
                            diags.push(
                                Diagnostic::error("segment id cannot be empty")
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
/// explicit, segment and action block — because every one of them can end
/// up as a file name, so the check must not be scoped to the explicit ones.
fn record_id(seen: &mut HashSet<String>, id: &str, span: SourceSpan, diags: &mut Vec<Diagnostic>) {
    if let Some(msg) = id_error(id) {
        diags.push(Diagnostic::error(msg).at(span).with_help(ID_HELP));
    }
    if !seen.insert(id.to_string()) {
        diags.push(
            Diagnostic::error(format!("duplicate segment id `{id}`"))
                .at(span)
                .with_help("give one of them an explicit unique `{#id}`"),
        );
    }
}
