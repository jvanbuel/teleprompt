use std::collections::BTreeMap;

use crate::ast::{ActionBlock, Chapter, Directive, Line, Node, Script};
use crate::attrs::{BlockAttrs, LineAttrs};
use crate::config::{Config, PartialConfig};
use crate::policy::{Align, PolicyKind};
use crate::DurationMs;
use crate::{BlockId, Diagnostic, Diagnostics, Hash, LineId, SourceSpan};

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
    /// What resolving found worth saying that does not stop the script,
    /// such as a bold label that names nobody in the cast.
    pub warnings: Vec<Diagnostic>,
}

#[derive(Debug, Clone)]
pub enum Element {
    Narration {
        id: LineId,
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
        /// Who says it, from the cast; the narrator when `None`.
        speaker: Option<String>,
        /// Source location of the paragraph this narration came from, so
        /// downstream diagnostics can point at the real line instead of a
        /// fabricated one.
        span: SourceSpan,
    },
    Action {
        block_id: BlockId,
        scene: String,
        body: String,
        /// The `include=` attribute's raw value, if present. Resolved
        /// against the script's directory by `teleprompt-compile`, which is
        /// where filesystem access is allowed (core stays pure).
        include: Option<String>,
        config: Config,
        policy: PolicyKind,
        align: Align,
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
        /// `stretch=`: its shots run this many times their own length.
        stretch: Option<f64>,
        /// `budget=`: a `fit-line` item's length, for a shot of unknown length.
        budget: Option<DurationMs>,
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
    if let Some(problem) = crate::config::locale_problem(locale) {
        return Err(Diagnostics(vec![Diagnostic::error(problem)]));
    }
    let id_diags = crate::ident::check_ids(script);
    if id_diags.iter().any(Diagnostic::is_error) {
        return Err(Diagnostics(id_diags));
    }
    let mut diags = Vec::new();
    let front = config_layer(&script.front_matter, "front matter", &mut diags);
    // Each layer with its section for this locale over it.
    let project = project.in_locale(locale);
    let front = front.in_locale(locale);
    let base = Config::merged(&[project.clone(), front.clone(), vec![cli.clone()]].concat());
    let mut r = Resolver {
        project: &project,
        front,
        cli,
        diags,
        config_problems: BTreeMap::new(),
        elements: Vec::new(),
        warnings: Vec::new(),
        last_speaker: None,
    };
    let mut chapters = Vec::new();

    for (chapter_index, chapter) in script.chapters.iter().enumerate() {
        chapters.push(ChapterInfo {
            slug: chapter.slug.clone(),
            title: chapter.title.clone(),
        });
        let chapter_cfg = [
            heading_layer(chapter, &mut r.diags),
            config_layer(
                &chapter.front_matter,
                &format!("chapter `{}` front matter", chapter.slug),
                &mut r.diags,
            ),
        ];
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
        warnings,
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
        warnings,
    })
}

/// A chapter's heading settings, `# Interview {speaker=guest}`, as a layer.
fn heading_layer(chapter: &Chapter, diags: &mut Vec<Diagnostic>) -> PartialConfig {
    if chapter.raw_attrs.trim().is_empty() {
        return PartialConfig::default();
    }
    let (settings, mut d) = crate::attrs::heading_attrs(&chapter.raw_attrs, chapter.span);
    diags.append(&mut d);
    match PartialConfig::from_settings(&settings) {
        Ok(c) => c,
        Err(crate::config::ConfigError::Yaml(e)) => {
            diags.push(
                Diagnostic::error(format!("chapter `{}`: {e}", chapter.slug))
                    .at(chapter.span)
                    .with_help(
                        "a heading takes the settings front matter does, dotted, \
                     such as {speaker=guest voice.speed=1.1}",
                    ),
            );
            PartialConfig::default()
        }
        Err(e) => {
            diags.push(
                Diagnostic::error(format!("chapter `{}`: {e}", chapter.slug)).at(chapter.span),
            );
            PartialConfig::default()
        }
    }
}

/// The cast member `label` names, as its slug: `Guest` is `guest`, and
/// `Ada Lovelace` is `ada-lovelace`.
fn cast_member(label: &str, config: &Config) -> Option<String> {
    let slug = crate::ast::slugify(label);
    config
        .voices
        .keys()
        .find(|name| crate::ast::slugify(name) == slug)
        .cloned()
}

/// A bold label that names nobody in the cast, so is read aloud.
fn not_a_speaker(id: &LineId, label: &str, config: &Config, span: SourceSpan) -> Diagnostic {
    let cast: Vec<String> = config.voices.keys().map(|k| format!("`{k}`")).collect();
    Diagnostic::warning(format!(
        "line `{id}` opens with `{label}:`, like a speaker, but the cast is {}: \
         it is read aloud",
        cast.join(", ")
    ))
    .at(span)
    .with_help(format!(
        "add `[voices.{}]` to the cast, or write `{label}:` without bold to say it",
        crate::ast::slugify(label)
    ))
}

/// A speaker the cast does not have, and who it does.
fn unknown_speaker(name: &str, config: &Config, span: SourceSpan) -> Diagnostic {
    let cast: Vec<String> = config.voices.keys().map(|k| format!("`{k}`")).collect();
    let message = if cast.is_empty() {
        format!("no speaker `{name}`: the script has no cast")
    } else {
        format!("no speaker `{name}` in the cast ({})", cast.join(", "))
    };
    Diagnostic::error(message).at(span).with_help(format!(
        "add `[voices.{name}]` to teleprompt.toml, or `voices:` to the front matter"
    ))
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
    project: &'a [PartialConfig],
    front: Vec<PartialConfig>,
    cli: &'a PartialConfig,
    diags: Vec<Diagnostic>,
    /// Merged-config problems, keyed by message so one bad `voice.speed` in
    /// front matter produces one diagnostic rather than one per paragraph.
    /// The span is the first line that resolved to the offending value —
    /// the merge has already flattened the layers, so which layer supplied
    /// it is not recoverable here, but the line it reaches is.
    config_problems: BTreeMap<String, SourceSpan>,
    elements: Vec<Element>,
    warnings: Vec<Diagnostic>,
    /// Who said the last line, once there is one: `Some(None)` for the
    /// narrator.
    last_speaker: Option<Option<String>>,
}

impl Resolver<'_> {
    fn merged(&self, chapter_cfg: &[PartialConfig], item: PartialConfig) -> Config {
        let layers = [
            self.project.to_vec(),
            self.front.clone(),
            chapter_cfg.to_vec(),
            vec![item, self.cli.clone()],
        ];
        Config::merged(&layers.concat())
    }

    fn resolve_line(
        &mut self,
        seg: &Line,
        chapter: &Chapter,
        chapter_index: usize,
        chapter_cfg: &[PartialConfig],
    ) {
        let (attrs, mut d) = LineAttrs::parse(&seg.raw_attrs, seg.span);
        self.diags.append(&mut d);
        let item = PartialConfig::from_line(&attrs);
        // Who says it: the cast member its label names, or the chapter's or
        // script's `speaker:`. Their voice is a layer over the chapter's,
        // under the line's own attributes. A label naming nobody is text.
        let around = self.merged(chapter_cfg, PartialConfig::default());
        let (speaker, text) = match &seg.label {
            Some(label) => match cast_member(label, &around) {
                Some(name) => (Some(name), seg.text.clone()),
                None => {
                    if !around.voices.is_empty() {
                        let w = not_a_speaker(&seg.id, label, &around, seg.span);
                        self.warnings.push(w);
                    }
                    (around.speaker.clone(), format!("{label}: {}", seg.text))
                }
            },
            None => (around.speaker.clone(), seg.text.clone()),
        };
        let cast = match &speaker {
            None => PartialConfig::default(),
            Some(name) => match around.voices.get(name) {
                Some(voice) => PartialConfig {
                    voice: Some(voice.clone()),
                    ..PartialConfig::default()
                },
                None => {
                    self.diags.push(unknown_speaker(name, &around, seg.span));
                    PartialConfig::default()
                }
            },
        };
        let layers = [
            self.project.to_vec(),
            self.front.clone(),
            chapter_cfg.to_vec(),
            vec![cast, item, self.cli.clone()],
        ];
        let mut config = Config::merged(&layers.concat());
        for problem in config.problems() {
            self.config_problems.entry(problem).or_insert(seg.span);
        }
        // Someone answering: the turn's pause before their line.
        if self
            .last_speaker
            .replace(speaker.clone())
            .is_some_and(|last| last != speaker)
        {
            let t = &mut config.timing;
            t.lead_in_ms =
                DurationMs::new(t.lead_in_ms.ms() + t.turn_gap_ms.ms()).unwrap_or(DurationMs::MAX);
        }
        self.elements.push(Element::Narration {
            id: seg.id.clone(),
            source_hash: Hash::of(text.as_bytes()),
            chapter: chapter.slug.clone(),
            chapter_index,
            text,
            config,
            speaker,
            span: seg.span,
        });
    }

    fn resolve_action_block(&mut self, block: &ActionBlock, chapter_cfg: &[PartialConfig]) {
        let (attrs, mut d) = BlockAttrs::parse(&block.info, block.span);
        self.diags.append(&mut d);
        let Some(scene) = attrs.scene.clone() else {
            self.diags.push(
                Diagnostic::error("action block has no `scene`")
                    .at(block.span)
                    .with_help(
                        "write ```teleprompt scene=<adapter or declared scene>, e.g. scene=vhs",
                    ),
            );
            return;
        };
        let policy = attrs.policy.unwrap_or(PolicyKind::Hold);
        if let (Some(_), false) = (attrs.align, policy == PolicyKind::Concurrent) {
            self.diags.push(
                Diagnostic::error(format!("`align` has no effect with `policy={policy}`"))
                    .at(block.span)
                    .with_help(
                        "align places a `concurrent` action against its line; \
                         write `policy=concurrent`, or drop `align`",
                    ),
            );
        }
        if let (Some(_), PolicyKind::FitAction | PolicyKind::TrimAction) = (attrs.stretch, policy) {
            self.diags.push(
                Diagnostic::error(format!("`stretch` and `policy={policy}` both set the pace"))
                    .at(block.span)
                    .with_help(
                        "`fit-action` and `trim-action` pace the action to its line; \
                         `stretch` paces it as you say. Keep one",
                    ),
            );
        }
        if let (Some(_), false) = (attrs.budget, policy == PolicyKind::FitLine) {
            self.diags.push(
                Diagnostic::error(format!(
                    "`budget` has no effect with `policy={policy}`: it is a `fit-line` item's length"
                ))
                    .at(block.span)
                    .with_help(
                        "a budget is the length of an item its picture leads; \
                         write `policy=fit-line`, or drop `budget`",
                    ),
            );
        }
        let config = self.merged(chapter_cfg, PartialConfig::from_block(&attrs));
        self.elements.push(Element::Action {
            block_id: block.id.clone(),
            scene,
            body: block.body.clone(),
            include: attrs.include,
            review: attrs.review,
            policy,
            align: attrs.align.unwrap_or(Align::Start),
            cue: attrs.cue,
            session: attrs.session,
            stretch: attrs.stretch,
            budget: attrs.budget,
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
    let error = |problem: String| {
        let speed = problem.starts_with("voice.speed");
        let d = Diagnostic::error(problem);
        if speed {
            d.with_help("voice.speed scales narration duration; 1.0 is unmodified")
        } else {
            d
        }
    };
    for problem in base.problems() {
        if !config_problems.contains_key(&problem) {
            diags.push(error(problem));
        }
    }
    for (problem, span) in config_problems {
        diags.push(error(problem).at(span));
    }
}
