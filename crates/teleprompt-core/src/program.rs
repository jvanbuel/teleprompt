use std::collections::BTreeMap;

use crate::ast::{ActionBlock, Chapter, Directive, Line, Node, Script};
use crate::attrs::{BlockAttrs, LineAttrs};
use crate::config::{Config, PartialConfig};
use crate::{Diagnostic, Diagnostics, Hash, SourceSpan};

/// A chapter as the manifest and other consumers need it: identity and a
/// human title. `resolve` flattens chapters away, so without this the
/// script's structure would not survive compilation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterInfo {
    pub slug: String,
    pub title: String,
}

/// A single locale's script, flattened from its chapter tree into one
/// ordered list of elements with configuration fully resolved per item.
#[derive(Debug, Clone)]
pub struct Program {
    pub script_name: String,
    pub locale: String,
    pub config: Config,
    /// Every chapter in document order, whether or not it contains
    /// narration.
    pub chapters: Vec<ChapterInfo>,
    pub elements: Vec<Element>,
}

#[derive(Debug, Clone)]
pub enum Element {
    Narration {
        id: String,
        text: String,
        source_hash: Hash,
        /// Slug of the chapter this paragraph belongs to. Never empty:
        /// content before the first heading is rejected at parse time.
        ///
        /// Human-facing identity only. Slugs derive from titles, cannot be
        /// pinned, and are never deduplicated, so two `# Setup` chapters
        /// share one slug — join on `chapter_index` instead.
        chapter: String,
        /// Position of that chapter in [`Program::chapters`]. The join key
        /// for anything that has to tell two identically-titled chapters
        /// apart.
        chapter_index: usize,
        config: Config,
        /// Source location of the paragraph this narration came from, so
        /// downstream diagnostics can point at the real line instead of a
        /// fabricated one.
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
        /// `cue="…"`: the phrase in the narration this shot starts on.
        ///
        /// A cue in the plain sense — the words that trigger what happens
        /// next. The thing triggered is the shot; this is what triggers it.
        cue: Option<String>,
        /// `session="…"`: which run of the scene this block belongs to.
        ///
        /// `None` is the scene's own session, which is the ordinary case:
        /// blocks naming a scene continue the screen the previous one left
        /// behind. A name here starts — or rejoins — a different run.
        session: Option<String>,
        /// `review=pending` on a block `from` generated: the command in
        /// it came out of someone else's document and has been read by
        /// nobody. Carried to `compile`, which is where `check`'s warnings
        /// are collected.
        review: Option<String>,
        /// Source location of the fence this action block came from, so a
        /// scene adapter's diagnostics point at the real line.
        span: SourceSpan,
    },
    Pause {
        ms: crate::DurationMs,
    },
}

/// Flattens `script`'s chapters into one ordered [`Element`] list, applying the
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
    let front = config_layer(&script.front_matter, "front matter", &mut diags);
    let base = Config::merged(&[project.clone(), front.clone(), cli.clone()]);
    let mut r = Resolver {
        project,
        front,
        cli,
        diags,
        config_problems: BTreeMap::new(),
        elements: Vec::new(),
    };
    let mut chapters = Vec::new();

    for (chapter_index, chapter) in script.chapters.iter().enumerate() {
        chapters.push(ChapterInfo {
            slug: chapter.slug.clone(),
            title: chapter.title.clone(),
        });
        let chapter_cfg = config_layer(
            &chapter.front_matter,
            &format!("chapter `{}` front matter", chapter.slug),
            &mut r.diags,
        );
        for node in &chapter.nodes {
            match node {
                Node::Line(seg) => r.resolve_line(seg, chapter, chapter_index, &chapter_cfg),
                Node::ActionBlock(block) => r.resolve_action_block(block, &chapter_cfg),
                Node::Directive(Directive::Pause(ms)) => {
                    r.elements.push(Element::Pause { ms: *ms });
                }
            }
        }
    }

    let Resolver {
        mut diags,
        config_problems,
        elements,
        ..
    } = r;
    report_config_problems(&base, config_problems, &mut diags);

    let d = Diagnostics(diags);
    if d.has_errors() {
        return Err(d);
    }

    Ok(Program {
        script_name: script_name.to_string(),
        locale: locale.to_string(),
        config: base,
        chapters,
        elements,
    })
}

/// Parses one YAML config layer, reporting a parse failure under `what`
/// and falling back to an empty layer.
fn config_layer(yaml: &str, what: &str, diags: &mut Vec<Diagnostic>) -> PartialConfig {
    match PartialConfig::from_yaml(yaml) {
        Ok(c) => c,
        Err(e) => {
            diags.push(Diagnostic::error(format!("{what}: {e}")));
            PartialConfig::default()
        }
    }
}

/// The script-wide config layers and everything `resolve` collects while
/// walking the chapters.
struct Resolver<'a> {
    project: &'a PartialConfig,
    front: PartialConfig,
    cli: &'a PartialConfig,
    diags: Vec<Diagnostic>,
    /// Merged-config problems, keyed by message so one bad `voice.speed` in
    /// front matter produces one diagnostic rather than one per paragraph.
    /// The span is the first line that resolved to the offending value —
    /// the merge has already flattened the layers, so which layer supplied
    /// it is not recoverable here, but the line it reaches is.
    config_problems: BTreeMap<String, SourceSpan>,
    elements: Vec<Element>,
}

impl Resolver<'_> {
    fn merged(&self, chapter_cfg: &PartialConfig, item: PartialConfig) -> Config {
        Config::merged(&[
            self.project.clone(),
            self.front.clone(),
            chapter_cfg.clone(),
            item,
            self.cli.clone(),
        ])
    }

    fn resolve_line(
        &mut self,
        seg: &Line,
        chapter: &Chapter,
        chapter_index: usize,
        chapter_cfg: &PartialConfig,
    ) {
        let (attrs, mut d) = LineAttrs::parse(&seg.raw_attrs, seg.span);
        self.diags.append(&mut d);
        let config = self.merged(chapter_cfg, PartialConfig::from_line(&attrs));
        for problem in config.problems() {
            self.config_problems.entry(problem).or_insert(seg.span);
        }
        let text = seg.text.clone();
        self.elements.push(Element::Narration {
            id: seg.id.clone().unwrap_or_default(),
            source_hash: Hash::of(text.as_bytes()),
            chapter: chapter.slug.clone(),
            chapter_index,
            text,
            config,
            span: seg.span,
        });
    }

    fn resolve_action_block(&mut self, block: &ActionBlock, chapter_cfg: &PartialConfig) {
        let (attrs, mut d) = BlockAttrs::parse(&block.info, block.span);
        self.diags.append(&mut d);
        let Some(scene) = attrs.scene.clone() else {
            self.diags.push(
                Diagnostic::error("action block has no `scene`")
                    .at(block.span)
                    .with_help("write ```teleprompt scene=browser"),
            );
            return;
        };
        let config = self.merged(chapter_cfg, PartialConfig::from_block(&attrs));
        self.elements.push(Element::Action {
            block_id: block.id.clone().unwrap_or_default(),
            scene,
            body: block.body.clone(),
            include: attrs.include,
            review: attrs.review,
            policy: attrs.policy.unwrap_or_else(|| "hold".to_string()),
            align: attrs.align.unwrap_or_else(|| "start".to_string()),
            cue: attrs.cue,
            session: attrs.session,
            config,
            span: block.span,
        });
    }
}

/// Reports each config problem once: at the first line it reached, or, for
/// a problem only the script-level merge has, unspanned so a value nothing
/// narrates against is still reported rather than sitting in the config
/// unread. Unspanned ones come first.
fn report_config_problems(
    base: &Config,
    config_problems: BTreeMap<String, SourceSpan>,
    diags: &mut Vec<Diagnostic>,
) {
    const HELP: &str = "voice.speed scales narration duration; 1.0 is unmodified";
    for problem in base.problems() {
        if !config_problems.contains_key(&problem) {
            diags.push(Diagnostic::error(problem).with_help(HELP));
        }
    }
    for (problem, span) in config_problems {
        diags.push(Diagnostic::error(problem).at(span).with_help(HELP));
    }
}
