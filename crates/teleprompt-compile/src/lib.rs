//! Where core, scene, voice and schedule meet (docs/design.md#crates):
//! walks a resolved [`Program`], has the adapters validate and split action
//! blocks, takes each line's duration from the voice cache or a
//! [`DurationEstimator`], pairs lines with the shots that follow them, and
//! schedules the items into a [`Timeline`]. It never reaches a voice
//! backend; see [`VoiceContext`].

use std::collections::BTreeMap;
use std::path::{Component, Path};

use teleprompt_cache::{CacheKey, VoiceCache};
use teleprompt_core::config::{default_adapter, Config, OutputConfig, SceneConfig};
use teleprompt_core::policy::Align;
use teleprompt_core::program::{ChapterInfo, Element, Program};
use teleprompt_core::voice::spoken;
use teleprompt_core::{
    BlockId, Diagnostic, Diagnostics, DurationMs, DurationSource, Hash, ItemId, LineId, PolicyKind,
    ShotId, SourceSpan,
};
use teleprompt_scene::{BlockSource, BodyOrigin, Measured, SceneCompiler, SceneRegistry, Shot};
use teleprompt_schedule::{schedule, ActionInput, Item, NarrationInput, Pacing, Policy, Timeline};
use teleprompt_voice::takes::{TakeMeta, Takes};
use teleprompt_voice::{DurationEstimator, SynthRequest, WordTiming};

pub mod length;
pub mod manifest;

/// Everything `compile` needs about voice, and deliberately no backend, so
/// the inner loop cannot synthesize (docs/design.md#async-boundary).
pub struct VoiceContext<'a> {
    pub backend_id: &'a str,
    pub backend_version: &'a str,
    /// The version of each other backend a line may pick, by id, for a
    /// cast whose speakers use several.
    pub other_backends: BTreeMap<String, String>,
    pub cache: &'a VoiceCache,
    pub estimator: &'a dyn DurationEstimator,
    /// Recorded takes; a current one stands in for synthesis.
    pub takes: &'a Takes,
}

/// An included file as an author would find it from where they invoked
/// teleprompt: `steps.mock`, not `./steps.mock`.
fn display_path(path: &Path) -> String {
    let s = path.display().to_string();
    s.strip_prefix("./").unwrap_or(&s).to_string()
}

/// Per-line detail the manifest needs and the [`Timeline`], a review
/// surface, deliberately leaves out: the prose and word timings.
#[derive(Debug, Clone)]
pub struct NarrationDetail {
    pub line_id: LineId,
    pub text: String,
    /// The chapter's slug, for display. Two chapters with the same title
    /// share one, so it is not a join key.
    pub chapter: String,
    /// The chapter's position in [`CompileOutput::chapters`]: the join key.
    pub chapter_index: usize,
    /// The request the timeline's duration came from. `dub` synthesizes
    /// from this rather than building its own, so the published duration
    /// and the audio file cannot disagree.
    pub synth_request: SynthRequest,
    /// The key `synth_request` was looked up under, and that `dub` stores
    /// its audio under.
    pub cache_key: CacheKey,
    pub word_timings: Option<Vec<WordTiming>>,
    /// The recorded take the line is spoken from, when it has a current
    /// one; `synth_request` and `cache_key` then go unused.
    pub take: Option<TakeMeta>,
    /// The voice backend that speaks it, by id.
    pub backend: String,
    /// Who says it, from the cast; the narrator when `None`.
    pub speaker: Option<String>,
}

impl VoiceContext<'_> {
    /// The version of backend `id`, if this compile has it.
    pub fn version_of(&self, id: &str) -> Option<&str> {
        if id == self.backend_id {
            Some(self.backend_version)
        } else {
            self.other_backends.get(id).map(String::as_str)
        }
    }
}

/// One action shot's source, as the adapter split it (and re-timed it), for
/// whatever draws the scene. Not in the manifest: it is adapter-native code.
#[derive(Debug, Clone)]
pub struct ShotSource {
    pub id: ShotId,
    pub scene: String,
    pub adapter: String,
    pub source: String,
}

#[derive(Debug)]
pub struct CompileOutput {
    pub timeline: Timeline,
    pub warnings: Vec<String>,
    /// One entry per narration line, in document order.
    pub narration: Vec<NarrationDetail>,
    /// Every action shot's source, in document order.
    pub shots: Vec<ShotSource>,
    // `chapters`, `output` and `scenes` are carried because `compile` drops
    // the `Program` and callers have no other route to them. `output` and
    // `scenes` stay out of the manifest: they are not timing facts.
    /// The script's chapters, in document order.
    pub chapters: Vec<ChapterInfo>,
    /// The frame the script asked for, resolved through every layer.
    pub output: OutputConfig,
    /// The scenes as configured, resolved through every layer.
    pub scenes: BTreeMap<String, SceneConfig>,
}

/// Where in a narration a cued action starts: from word timings when the
/// backend gave them ([`word_offset_ms`]), otherwise interpolated by
/// characters (docs/design.md#cues).
fn at_offset_ms(
    phrase: &str,
    text: &str,
    narration: Option<&NarrationInput>,
    policy: PolicyKind,
    timed: Option<(&[WordTiming], &BTreeMap<String, String>)>,
) -> Result<Option<u64>, Diagnostic> {
    if policy != PolicyKind::Concurrent {
        return Err(Diagnostic::error(format!(
            "`cue=\"{phrase}\"` needs `policy=concurrent`, not `{policy}`"
        ))
        .with_help(
            "hold runs the action after the narration, and fit-action and trim-action \
             size it to fit; a shot only means something where the two run together",
        ));
    }
    let Some(narration) = narration else {
        return Err(
            Diagnostic::error(format!("`cue=\"{phrase}\"` has no narration to start in"))
                .with_help("a shot names a phrase in the paragraph above the block"),
        );
    };

    let Some(at) = text.find(phrase) else {
        return Err(Diagnostic::error(format!(
            "`cue=\"{phrase}\"` is not in the narration above it"
        ))
        .with_help(format!("the paragraph reads: {text}")));
    };

    if text.is_empty() {
        return Ok(None);
    }
    if let Some((words, pronounce)) = timed {
        if let Some(ms) =
            word_offset_ms(&spoken(phrase, pronounce), &spoken(text, pronounce), words)
        {
            return Ok(Some(ms));
        }
    }
    let fraction = text[..at].chars().count() as f64 / text.chars().count() as f64;
    Ok(Some(
        (narration.duration_ms as f64 * fraction).round() as u64
    ))
}

/// When the first word of `phrase` is said, from the backend's timings of
/// `text`, both as the voice was given them (pronunciations applied).
///
/// Words compare lowercased without punctuation. If the timed words match
/// the text's one for one, the word is taken by position; otherwise (a
/// backend that reads `0:12` as three words) it is the timed occurrence
/// nearest the same relative position. `None` when the word was not timed.
pub fn word_offset_ms(phrase: &str, text: &str, words: &[WordTiming]) -> Option<u64> {
    fn norm(w: &str) -> String {
        w.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    }
    let said: Vec<String> = text
        .split_whitespace()
        .map(norm)
        .filter(|w| !w.is_empty())
        .collect();
    let wanted: Vec<String> = phrase
        .split_whitespace()
        .map(norm)
        .filter(|w| !w.is_empty())
        .collect();
    let first = wanted.first()?;
    let at = (0..said.len()).find(|i| said[*i..].starts_with(&wanted))?;
    let timed: Vec<String> = words.iter().map(|w| norm(&w.word)).collect();

    if timed.len() == said.len() && timed[at] == *first {
        return Some(words[at].start_ms);
    }
    let relative = at as f64 / said.len().max(1) as f64;
    (0..timed.len())
        .filter(|j| timed[*j] == *first)
        .min_by(|a, b| {
            let d = |j: &usize| (*j as f64 / timed.len() as f64 - relative).abs();
            d(a).total_cmp(&d(b))
        })
        .map(|j| words[j].start_ms)
}

/// Has each adapter rewrite a shot whose scheduled length differs from its
/// own estimate, so the source captured is the one that fits its slot.
///
/// The shot's hash moves with the source, which is how slot length reaches
/// the capture key (docs/design.md#capture-key). Where an adapter cannot
/// re-time, the source stands and the renderer holds the last frame.
fn retime_stretched_shots(
    timeline: &mut Timeline,
    shots: &mut [ShotSource],
    registry: &SceneRegistry,
) {
    for entry in &mut timeline.entries {
        let Some(action) = entry.action.as_mut() else {
            continue;
        };
        let Some(published) = shots.iter_mut().find(|s| s.id == action.shot) else {
            continue;
        };
        let Some(adapter) = registry.get(&action.adapter) else {
            continue;
        };

        // Only `fit-action` and `trim-action` change the number.
        let shot = Shot {
            id: published.id.clone(),
            source: published.source.clone(),
            hash: action.shot_hash,
            index: 0,
        };
        if adapter.estimate(&shot).duration_ms() == Some(action.duration_ms.ms()) {
            continue;
        }

        if let Some(source) = adapter.retime(&shot, action.duration_ms.ms()) {
            action.shot_hash = Hash::of(source.as_bytes());
            action.duration_source = DurationSource::Exact;
            published.source = source;
        }
    }
}

/// The scene name a pause wears, which is not a scene and has no picture.
const PAUSE_SCENE: &str = "pause";

/// The capture recipe version, part of every capture key. Bump it whenever
/// the picture a backend draws changes for the same script: a new renderer
/// version, a change to the tape or script teleprompt writes, new window
/// chrome, or a changed default.
///
/// Neither the adapter name nor the settings cover this. The adapter names
/// the scene language, not the program that rasterises it, and settings are
/// only what the author wrote, so a changed default moves neither and stale
/// clips would be served as current.
pub const CAPTURE_RECIPE: &str = "vhs-0.11-pw-1.63-v2";

/// Names each shot's picture, which is its own source chained to every shot
/// before it in its session (docs/design.md#capture-key).
///
/// Runs after [`retime_stretched_shots`], so the chain is built from the
/// source that will actually be captured.
fn chain_capture_keys(
    timeline: &mut Timeline,
    config: &Config,
    registry: &SceneRegistry,
    shots: &[ShotSource],
) {
    let mut chains: BTreeMap<(String, String), Hash> = BTreeMap::new();
    let mut inputs: BTreeMap<String, String> = BTreeMap::new();

    for entry in &mut timeline.entries {
        let Some(action) = entry.action.as_mut() else {
            continue;
        };
        // A pause holds whatever is on screen, so it joins no chain: later
        // keys must not depend on how long it was.
        if action.scene == PAUSE_SCENE {
            action.capture_key = action.shot_hash;
            continue;
        }

        // name(n): recipe, adapter, scene settings, inputs, shot source.
        let implicit = SceneConfig {
            adapter: action.adapter.clone(),
            settings: BTreeMap::new(),
        };
        let scene = Some(config.scenes.get(&action.scene).unwrap_or(&implicit));
        let settings = scene
            .map(SceneConfig::settings_fingerprint)
            .unwrap_or_default();
        // Scene-wide inputs are read once per scene.
        let inputs = inputs
            .entry(action.scene.clone())
            .or_insert_with(|| match (scene, registry.get(&action.adapter)) {
                (Some(scene), Some(adapter)) => fingerprint(&adapter.inputs(scene)),
                _ => String::new(),
            })
            .clone();
        let own = match (scene, registry.get(&action.adapter)) {
            (Some(scene), Some(adapter)) => shots
                .iter()
                .find(|s| s.id == action.shot)
                .map(|s| fingerprint(&adapter.shot_inputs(scene, &s.source)))
                .unwrap_or_default(),
            _ => String::new(),
        };
        let shot_hash = action.shot_hash.to_string();
        let mut fields = vec![CAPTURE_RECIPE, &action.adapter, &settings];
        // Empty inputs are left out, so they do not change the key.
        if !inputs.is_empty() {
            fields.push(&inputs);
        }
        if !own.is_empty() {
            fields.push(&own);
        }
        fields.push(&shot_hash);
        let name = Hash::of_fields(&fields);

        // An adapter whose shots do not continue is keyed by name(n) alone.
        if registry
            .get(&action.adapter)
            .is_some_and(|adapter| !adapter.continues())
        {
            action.capture_key = Hash::of_fields(&[&name.to_string()]);
            continue;
        }

        // One chain per (scene, session). The session name is not hashed:
        // two sessions that open with the same shots share clips until
        // they diverge.
        let key = (
            action.scene.clone(),
            action.session.clone().unwrap_or_default(),
        );
        let chain = match chains.get(&key) {
            None => Hash::of_fields(&[&name.to_string()]),
            Some(previous) => Hash::of_fields(&[&previous.to_string(), &name.to_string()]),
        };
        chains.insert(key, chain);
        action.capture_key = chain;
    }
}

/// A hash of the contents of `paths`, as [`SceneCompiler::inputs`]
/// describes them: directories recursively in a stable order, skipping
/// `node_modules` and dot-entries. Empty when there is nothing to read.
///
/// [`SceneCompiler::inputs`]: teleprompt_scene::SceneCompiler::inputs
fn fingerprint(paths: &[std::path::PathBuf]) -> String {
    fn walk(path: &Path, out: &mut Vec<String>) {
        if path.is_dir() {
            let Ok(entries) = std::fs::read_dir(path) else {
                return;
            };
            let mut children: Vec<_> = entries.flatten().map(|e| e.path()).collect();
            children.sort();
            for child in children {
                let name = child.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name != "node_modules" && !name.starts_with('.') {
                    walk(&child, out);
                }
            }
        } else if let Ok(hash) = Hash::of_file(path) {
            out.push(format!("{}:{hash}", path.display()));
        }
    }
    let mut files = Vec::new();
    for path in paths {
        walk(path, &mut files);
    }
    if files.is_empty() {
        return String::new();
    }
    let fields: Vec<&str> = files.iter().map(String::as_str).collect();
    Hash::of_fields(&fields).to_string()
}

/// Compiles `program` into a scheduled [`Timeline`].
///
/// Deterministic: the same inputs produce byte-identical timeline JSON.
/// Durations come from the voice cache on a hit and the estimator on a
/// miss (docs/design.md#estimated-and-measured).
pub fn compile(
    program: &Program,
    registry: &SceneRegistry,
    voice_ctx: &VoiceContext,
    base_dir: &Path,
    version: &str,
) -> Result<CompileOutput, Diagnostics> {
    let mut walker = Walker {
        program,
        registry,
        voice: voice_ctx,
        base_dir,
        diags: Vec::new(),
        items: Vec::new(),
        narration: Vec::new(),
        shots: Vec::new(),
        warnings: Vec::new(),
        pending: None,
    };
    for element in &program.elements {
        match element {
            Element::Narration { .. } => walker.narration(element),
            Element::Action {
                block_id,
                scene,
                body,
                include,
                config,
                policy,
                align,
                cue,
                session,
                stretch,
                budget,
                review,
                span,
            } => walker.action(&Block {
                block_id,
                scene,
                body,
                include: include.as_deref(),
                config,
                policy: *policy,
                align: *align,
                cue: cue.as_deref(),
                session,
                stretch: *stretch,
                budget: *budget,
                review: review.as_deref(),
                span,
            }),
            Element::Pause { ms } => walker.pause(*ms),
        }
    }
    walker.flush();

    let d = Diagnostics(walker.diags);
    if d.has_errors() {
        return Err(d);
    }

    let mut shot_sources = walker.shots;
    let (mut timeline, scheduling_warnings) = schedule(
        &walker.items,
        &program.script_name,
        &program.locale,
        version,
    );
    retime_stretched_shots(&mut timeline, &mut shot_sources, registry);
    chain_capture_keys(&mut timeline, &program.config, registry, &shot_sources);
    // Cache warnings first: they explain the scheduler's numbers.
    let mut warnings = walker.warnings;
    warnings.extend(scheduling_warnings);
    if let Some(length) = program.config.timing.length_ms {
        warnings.extend(length::over_length(
            &timeline,
            &walker.narration,
            &program.chapters,
            length.ms(),
        ));
    }
    Ok(CompileOutput {
        timeline,
        warnings,
        narration: walker.narration,
        shots: shot_sources,
        scenes: program.config.scenes.clone(),
        chapters: program.chapters.clone(),
        output: program.config.output.clone(),
    })
}

/// A line waiting to pair with the first shot of the next action block.
struct Pending {
    input: NarrationInput,
    id: LineId,
    /// What a `cue=` on that block searches.
    text: String,
    config: Config,
}

/// An action block's fields, borrowed from its [`Element::Action`].
struct Block<'e> {
    block_id: &'e BlockId,
    scene: &'e String,
    body: &'e str,
    include: Option<&'e str>,
    config: &'e Config,
    policy: PolicyKind,
    align: Align,
    cue: Option<&'e str>,
    session: &'e Option<String>,
    stretch: Option<f64>,
    budget: Option<DurationMs>,
    review: Option<&'e str>,
    span: &'e SourceSpan,
}

/// How a block's shots become items, once the block has checked out.
struct Placing<'a> {
    adapter_name: String,
    adapter: &'a dyn SceneCompiler,
    policy: Policy,
    cue_ms: Option<u64>,
    origin: BodyOrigin,
}

/// Walks a program's elements into schedulable items, collecting every
/// diagnostic rather than stopping at the first.
///
/// A block that fails leaves any pending line unpaired, so the line pairs
/// with the next block that compiles, exactly as if the failed block were
/// not there.
struct Walker<'a, 'v> {
    program: &'a Program,
    registry: &'a SceneRegistry,
    voice: &'a VoiceContext<'v>,
    base_dir: &'a Path,
    diags: Vec<Diagnostic>,
    items: Vec<Item>,
    narration: Vec<NarrationDetail>,
    shots: Vec<ShotSource>,
    warnings: Vec<String>,
    pending: Option<Pending>,
}

impl<'a> Walker<'a, '_> {
    /// A line no action block claimed becomes an item of its own.
    fn flush(&mut self) {
        if let Some(p) = self.pending.take() {
            self.items.push(Item {
                id: p.id.into(),
                narration: Some(p.input),
                action: None,
                policy: Policy::Hold,
                pacing: Pacing::from(&p.config),
            });
        }
    }

    fn narration(&mut self, element: &Element) {
        let Element::Narration {
            id,
            text,
            source_hash,
            chapter,
            chapter_index,
            config,
            speaker,
            span,
        } = element
        else {
            return;
        };
        let (source_hash, chapter_index) = (*source_hash, *chapter_index);
        self.flush();
        let backend = &config.voice.backend;
        let Some(backend_version) = self.voice.version_of(backend).map(str::to_string) else {
            self.diags.push(
                Diagnostic::error(format!(
                    "line `{id}` is spoken by voice backend `{backend}`, which this compile does not have"
                ))
                .at(*span),
            );
            return;
        };
        let req = SynthRequest {
            // docs/design.md#word-timings: the voice gets the
            // pronunciation, and it is in the cache key.
            text: spoken(text, &config.voice.pronounce),
            locale: self.program.locale.clone(),
            voice: config.voice.voice.clone(),
            speed: config.voice.speed,
            instruct: config.voice.instruct.clone(),
        };
        let cache_key = teleprompt_cache::key(backend, &backend_version, &req);
        let take = self.voice.takes.current(id, text).cloned();
        let (duration_ms, duration_source, word_timings, audio_hash) = match &take {
            Some(t) => (t.duration_ms, DurationSource::Measured, None, t.audio_hash),
            None => {
                let Some((ms, source, timings)) = self.duration(id, &cache_key, &req) else {
                    return;
                };
                // Identifies the audio this line resolves to, via its cache
                // key; not the manifest's hash of the WAV bytes.
                (
                    ms,
                    source,
                    timings,
                    Hash::of(cache_key.to_string().as_bytes()),
                )
            }
        };

        self.narration.push(NarrationDetail {
            line_id: id.clone(),
            text: text.to_string(),
            chapter: chapter.to_string(),
            chapter_index,
            synth_request: req,
            cache_key: cache_key.clone(),
            word_timings,
            take: take.clone(),
            backend: backend.clone(),
            speaker: speaker.clone(),
        });
        self.pending = Some(Pending {
            input: NarrationInput {
                line_id: id.clone(),
                source_hash,
                audio_hash,
                duration_ms,
                duration_source,
                recorded: take.is_some(),
                words: text.split_whitespace().count(),
                // From the line's own config: the item it joins may carry
                // the action block's config, which lacks a line-level
                // `lead_in=` or `tail=`.
                lead_in_ms: config.timing.lead_in_ms,
                tail_ms: config.timing.tail_ms,
            },
            id: id.clone(),
            text: text.to_string(),
            config: config.clone(),
        });
    }

    /// A line's duration: measured from the cache, or estimated on a miss.
    fn duration(
        &mut self,
        id: &str,
        key: &CacheKey,
        req: &SynthRequest,
    ) -> Option<(u64, DurationSource, Option<Vec<WordTiming>>)> {
        // Metadata only: reading the WAV on every hit would put the whole
        // cache's audio through `plan` on every run.
        let read = match self.voice.cache.lookup_meta(key) {
            Ok(c) => c,
            Err(e) => {
                self.diags
                    .push(Diagnostic::error(format!("line `{id}`: {e}")));
                return None;
            }
        };
        // An unreadable entry is a miss, but say so: otherwise the line
        // silently reverts to `estimated`.
        if let Some(w) = read.warning() {
            self.warnings.push(format!("line `{id}`: {w}"));
        }
        Some(match read.hit() {
            Some(hit) => (hit.duration_ms, DurationSource::Measured, hit.word_timings),
            None => (
                self.voice.estimator.estimate_ms(req),
                DurationSource::Estimated,
                None,
            ),
        })
    }

    fn action(&mut self, b: &Block) {
        // A block `from` drafted runs commands lifted from another document;
        // warn until a human removes the attribute.
        if b.review == Some("pending") {
            self.warnings.push(format!(
                "action block `{}` is marked `review=pending`: \
it was drafted from another document and has not been reviewed. \
Read it, then remove the attribute.",
                b.block_id
            ));
        }
        let Some((body, origin, fragment)) = self.load_body(b) else {
            return;
        };
        let cue_ms = match b.cue {
            None => None,
            Some(phrase) => match self.cue_offset(phrase, b.policy) {
                Ok(ms) => ms,
                Err(d) => {
                    self.diags.push(d.at(*b.span));
                    return;
                }
            },
        };
        let policy = Policy::new(b.policy, b.align);
        let Some((adapter_name, adapter)) = self.adapter_for(b.scene, b.config) else {
            return;
        };
        let Some(shots) = self.split(adapter, b, body, &origin, fragment.as_deref()) else {
            return;
        };
        if shots.is_empty() {
            // An all-`mark` block has no shots. A line pairs only with the
            // block immediately after it, so flush it.
            self.flush();
            return;
        }
        let placing = Placing {
            adapter_name,
            adapter,
            policy,
            cue_ms,
            origin,
        };
        self.push_shots(b, &shots, &placing);
    }

    /// The block's body, where its lines are numbered from, and the fragment
    /// an `include=file#fragment` names. `None` once the reason is reported.
    fn load_body(&mut self, b: &Block) -> Option<(String, BodyOrigin, Option<String>)> {
        // The fragment is the adapter's to interpret (see `split`).
        let (include, fragment) = match b.include.map(|i| i.split_once('#')) {
            Some(Some((path, frag))) => (Some(path), Some(frag.to_string())),
            _ => (b.include, None),
        };
        let Some(rel) = include else {
            return Some((
                b.body.to_string(),
                BodyOrigin::Inline { fence: *b.span },
                fragment,
            ));
        };
        // Path traversal: check the raw components before joining.
        // `base_dir.join(p).starts_with(base_dir)` never rejects `..`, and
        // canonicalising would follow a symlink out of the project.
        let p = Path::new(rel);
        if p.is_absolute() || p.components().any(|c| matches!(c, Component::ParentDir)) {
            self.diags.push(Diagnostic::error(format!(
                "included file `{rel}` resolves outside the project"
            )));
            return None;
        }
        if !b.body.trim().is_empty() {
            self.diags.push(Diagnostic::error(format!(
                "action block has both a body and an `include`: `{rel}`"
            )));
            return None;
        }
        let path = self.base_dir.join(rel);
        match std::fs::read_to_string(&path) {
            // Adapter diagnostics then name this file.
            Ok(s) => Some((
                s,
                BodyOrigin::Included {
                    path: display_path(&path),
                },
                fragment,
            )),
            Err(e) => {
                // Name the path as written and, if it differs, as resolved.
                let resolved = display_path(&path);
                let at = if resolved == rel {
                    String::new()
                } else {
                    format!(" ({resolved})")
                };
                self.diags.push(Diagnostic::error(format!(
                    "cannot read included file `{rel}`{at}: {e}"
                )));
                None
            }
        }
    }

    /// Where in the pending line a `cue=` puts the action's start.
    fn cue_offset(&self, phrase: &str, policy: PolicyKind) -> Result<Option<u64>, Diagnostic> {
        let pending = self.pending.as_ref();
        // The pending line's detail is the last one pushed.
        let timed = pending.and_then(|p| {
            self.narration
                .last()
                .and_then(|d| d.word_timings.as_deref())
                .map(|w| (w, &p.config.voice.pronounce))
        });
        let text = pending.map_or("", |p| p.text.as_str());
        at_offset_ms(phrase, text, pending.map(|p| &p.input), policy, timed)
    }

    /// The adapter configured for `scene`, or its default.
    fn adapter_for(
        &mut self,
        scene: &str,
        config: &Config,
    ) -> Option<(String, &'a dyn SceneCompiler)> {
        let declared = config.scenes.get(scene);
        let adapter_name =
            declared.map_or_else(|| default_adapter(scene).to_string(), |s| s.adapter.clone());
        let registry = self.registry;
        let Some(adapter) = registry.get(&adapter_name) else {
            let available = registry.available().join(", ");
            if declared.is_none() && adapter_name == scene {
                self.diags.push(
                    Diagnostic::error(format!("unknown scene `{scene}`")).with_help(format!(
                        "use an adapter's name as the scene ({available}), or declare \
                             it under [scene.{scene}] in teleprompt.toml with its adapter"
                    )),
                );
                return None;
            }
            self.diags.push(
                Diagnostic::error(format!(
                    "scene `{scene}` needs adapter `{adapter_name}`, but no adapter `{adapter_name}` is available"
                ))
                .with_help(format!("available adapters: {available}")),
            );
            return None;
        };
        Some((adapter_name, adapter))
    }

    /// Has the adapter validate the body, select its fragment, and split it
    /// into shots.
    fn split(
        &mut self,
        adapter: &dyn SceneCompiler,
        b: &Block,
        body: String,
        origin: &BodyOrigin,
        fragment: Option<&str>,
    ) -> Option<Vec<Shot>> {
        let src = BlockSource {
            scene: b.scene.clone(),
            body,
            origin: origin.clone(),
        };
        let mut validated = match adapter.validate(&src) {
            Ok(v) => v,
            Err(mut e) => {
                self.diags.append(&mut e);
                return None;
            }
        };
        if let Some(fragment) = fragment {
            match adapter.select(&validated.body, fragment) {
                Ok(part) => validated.body = part,
                Err(why) => {
                    self.diags.push(Diagnostic::error(why).at(*b.span));
                    return None;
                }
            }
        }
        match adapter.shots(&validated, b.block_id) {
            Ok(s) => Some(s),
            Err(mut e) => {
                self.diags.append(&mut e);
                None
            }
        }
    }

    /// One item per shot; the first takes the pending line, if there is one.
    fn push_shots(&mut self, b: &Block, shots: &[Shot], how: &Placing) {
        for (i, shot) in shots.iter().enumerate() {
            self.shots.push(ShotSource {
                id: shot.id.clone(),
                scene: b.scene.clone(),
                adapter: how.adapter_name.clone(),
                source: shot.source.clone(),
            });
            let Some(measured) = self
                .stretched(b, how, how.adapter.estimate(shot))
                .and_then(|m| self.budgeted(b, how, m))
            else {
                continue;
            };
            // An `Unknown` shot takes its line's length, so one with no line
            // would silently last no time at all.
            let narrated = i == 0 && self.pending.is_some();
            if measured == Measured::Unknown && !narrated {
                self.diags.push(
                    how.origin.locate(
                        Diagnostic::error(format!(
                            "`{}` has no sentence and states no length of its own, \
                         so it would last no time at all",
                            shot.id
                        ))
                        .with_help(format!(
                            "a `{}` shot lasts as long as the paragraph \
                         it follows: give it a paragraph and a block of its own \
                         rather than a mark",
                            how.adapter_name
                        )),
                        0,
                        0,
                    ),
                );
                continue;
            }
            let action = ActionInput {
                shot_id: shot.id.clone(),
                scene: b.scene.clone(),
                adapter: how.adapter_name.clone(),
                shot_hash: shot.hash,
                // Zero for `Unknown`: the scheduler substitutes the line's
                // length.
                duration_ms: measured.duration_ms().unwrap_or(0),
                duration_source: match measured {
                    Measured::Exact(_) => DurationSource::Exact,
                    Measured::Estimated(_) => DurationSource::Estimated,
                    Measured::Unknown => DurationSource::Unknown,
                },
                // Only the shot paired with the line can be cued.
                cue_ms: if i == 0 { how.cue_ms } else { None },
                session: b.session.clone(),
            };
            let paired = if i == 0 { self.pending.take() } else { None };
            let (id, narration) = match paired {
                Some(p) => (ItemId::from(p.id), Some(p.input)),
                None => (ItemId::from(shot.id.clone()), None),
            };
            self.items.push(Item {
                id,
                narration,
                action: Some(action),
                policy: how.policy,
                pacing: Pacing::from(b.config),
            });
        }
    }

    /// A shot's length with the block's `stretch=` applied. Only a shot that
    /// states its own length can be stretched: the adapter re-times it to
    /// the new length ([`retime_stretched_shots`]). `None` once reported.
    fn stretched(&mut self, b: &Block, how: &Placing, measured: Measured) -> Option<Measured> {
        let Some(factor) = b.stretch else {
            return Some(measured);
        };
        let timing = &b.config.timing;
        if !(timing.min_stretch..=timing.max_stretch).contains(&factor) {
            self.diags.push(
                Diagnostic::error(format!(
                    "`stretch={factor}` is outside {} to {}",
                    timing.min_stretch, timing.max_stretch
                ))
                .at(*b.span)
                .with_help("widen `min_stretch` or `max_stretch` to go further"),
            );
            return None;
        }
        let scale = |ms: u64| (ms as f64 * factor).round() as u64;
        match measured {
            Measured::Exact(ms) => Some(Measured::Exact(scale(ms))),
            Measured::Estimated(ms) => Some(Measured::Estimated(scale(ms))),
            Measured::Unknown => {
                self.diags.push(
                    Diagnostic::error(format!(
                        "a `{}` shot states no length of its own, so it cannot be stretched",
                        how.adapter_name
                    ))
                    .at(*b.span)
                    .with_help("it takes its line's length: lengthen the line instead"),
                );
                None
            }
        }
    }

    /// A `fit-line` shot's length: its own, or the block's `budget=` for
    /// one that states none. The picture leads, so it must have a length
    /// (docs/design.md#led-by-the-picture). `None` once reported.
    fn budgeted(&mut self, b: &Block, how: &Placing, measured: Measured) -> Option<Measured> {
        if b.policy != PolicyKind::FitLine {
            return Some(measured);
        }
        match (measured, b.budget) {
            (Measured::Unknown, Some(budget)) => Some(Measured::Exact(budget.ms())),
            (Measured::Unknown, None) => {
                self.diags.push(
                    Diagnostic::error(format!(
                        "`fit-line` fits the line to its picture, and a `{}` shot \
                         states no length",
                        how.adapter_name
                    ))
                    .at(*b.span)
                    .with_help("give the block the picture's length, e.g. `budget=6.5s`"),
                );
                None
            }
            (known, Some(_)) => {
                self.diags.push(
                    Diagnostic::error(format!(
                        "`budget` on a `{}` shot that states its own length ({} ms)",
                        how.adapter_name,
                        known.duration_ms().unwrap_or(0)
                    ))
                    .at(*b.span)
                    .with_help("the two could disagree: drop `budget`, it is for shots that state no length"),
                );
                None
            }
            (known, None) => Some(known),
        }
    }

    fn pause(&mut self, pause: DurationMs) {
        let ms = pause.ms();
        self.flush();
        let id = format!("pause-{}", self.items.len());
        self.items.push(Item {
            id: ItemId::new(id.clone()),
            narration: None,
            // A pause rides in the action slot, so the scheduler needs no
            // third case.
            action: Some(ActionInput {
                shot_id: ShotId::new(id),
                scene: PAUSE_SCENE.into(),
                adapter: PAUSE_SCENE.into(),
                shot_hash: Hash::of(ms.to_string().as_bytes()),
                duration_ms: ms,
                duration_source: DurationSource::Exact,
                cue_ms: None,
                session: None,
            }),
            policy: Policy::Hold,
            pacing: Pacing::from(&self.program.config),
        });
    }
}
