use crate::ast::{Directive, Node, Script};
use crate::attrs::{parse_attrs, BLOCK_KEYS, SEGMENT_KEYS};
use crate::config::{Config, PartialConfig};
use crate::{Diagnostic, Diagnostics, Hash, SourceSpan};

/// A single locale's script, flattened from its chapter tree into one
/// ordered list of items with configuration fully resolved per item.
#[derive(Debug, Clone)]
pub struct Program {
    pub script_name: String,
    pub locale: String,
    pub config: Config,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone)]
pub enum Item {
    Narration {
        id: String,
        text: String,
        source_hash: Hash,
        config: Config,
        /// Source location of the paragraph this narration came from
        /// (controller ruling F12), so downstream diagnostics can point at
        /// the real line instead of a fabricated one.
        span: SourceSpan,
    },
    Action {
        block_id: String,
        scene: String,
        body: String,
        /// The `include=` attribute's raw value, if present. Resolved
        /// against the script's directory by `teleprompt-compile`, which is
        /// where filesystem access is allowed (core stays pure).
        include: Option<String>,
        config: Config,
        policy: String,
        align: String,
        /// Source location of the fence this action block came from
        /// (controller ruling F12) — Task 11 passes it to the scene adapter
        /// for validation instead of inventing `line: 0`.
        span: SourceSpan,
    },
    Pause {
        ms: u64,
    },
}

/// Flattens `script`'s chapters into one ordered [`Item`] list, applying the
/// layered config merge (`[project, front matter, item attributes, cli]`)
/// per item, and computes each narration's `source_hash` over its normalised
/// text.
pub fn resolve(
    script: &Script,
    script_name: &str,
    locale: &str,
    project: &PartialConfig,
    cli: &PartialConfig,
) -> Result<Program, Diagnostics> {
    let mut diags = Vec::new();

    let front = match PartialConfig::from_yaml(&script.front_matter) {
        Ok(f) => f,
        Err(e) => {
            diags.push(Diagnostic::error(format!("front matter: {e}")));
            PartialConfig::default()
        }
    };

    let base = Config::merged(&[project.clone(), front.clone(), cli.clone()]);
    let mut items = Vec::new();

    for chapter in &script.chapters {
        let chapter_cfg = match PartialConfig::from_yaml(&chapter.front_matter) {
            Ok(c) => c,
            Err(e) => {
                diags.push(Diagnostic::error(format!(
                    "chapter `{}` front matter: {e}",
                    chapter.slug
                )));
                PartialConfig::default()
            }
        };

        for node in &chapter.nodes {
            match node {
                Node::Segment(seg) => {
                    let (attrs, mut d) = parse_attrs(&seg.raw_attrs, SEGMENT_KEYS, seg.span);
                    diags.append(&mut d);
                    let config = Config::merged(&[
                        project.clone(),
                        front.clone(),
                        chapter_cfg.clone(),
                        PartialConfig::from_attrs(&attrs),
                        cli.clone(),
                    ]);
                    let text = seg.text.clone();
                    items.push(Item::Narration {
                        id: seg.id.clone().unwrap_or_default(),
                        source_hash: Hash::of(text.as_bytes()),
                        text,
                        config,
                        span: seg.span,
                    });
                }
                Node::ActionBlock(block) => {
                    let (attrs, mut d) = parse_attrs(&block.info, BLOCK_KEYS, block.span);
                    diags.append(&mut d);
                    let Some(scene) = attrs.get("scene").map(str::to_string) else {
                        diags.push(
                            Diagnostic::error("action block has no `scene`")
                                .at(block.span)
                                .with_help("write ```teleprompt scene=browser"),
                        );
                        continue;
                    };
                    let config = Config::merged(&[
                        project.clone(),
                        front.clone(),
                        chapter_cfg.clone(),
                        PartialConfig::from_attrs(&attrs),
                        cli.clone(),
                    ]);
                    items.push(Item::Action {
                        block_id: block.id.clone().unwrap_or_default(),
                        scene,
                        body: block.body.clone(),
                        include: attrs.get("include").map(str::to_string),
                        policy: attrs.get("policy").unwrap_or("hold").to_string(),
                        align: attrs.get("align").unwrap_or("start").to_string(),
                        config,
                        span: block.span,
                    });
                }
                Node::Directive(Directive::Pause(ms)) => items.push(Item::Pause { ms: *ms }),
            }
        }
    }

    let d = Diagnostics(diags);
    if d.has_errors() {
        return Err(d);
    }

    Ok(Program {
        script_name: script_name.to_string(),
        locale: locale.to_string(),
        config: base,
        items,
    })
}
