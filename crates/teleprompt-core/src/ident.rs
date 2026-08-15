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

fn record_id(seen: &mut HashSet<String>, id: &str, span: SourceSpan, diags: &mut Vec<Diagnostic>) {
    if !seen.insert(id.to_string()) {
        diags.push(
            Diagnostic::error(format!("duplicate segment id `{id}`"))
                .at(span)
                .with_help("give one of them an explicit unique `{#id}`"),
        );
    }
}
