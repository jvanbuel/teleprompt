//! Walking a program's elements into schedulable items.

use std::collections::BTreeMap;
use std::path::{Component, Path};

use crate::schedule::{ActionInput, Item, NarrationInput, Pacing};
use teleprompt_core::policy::Align;
use teleprompt_core::voice::spoken;
use teleprompt_core::{
    Diagnostic, DurationMs, DurationSource, Hash, ItemId, LineId, PolicyKind, ShotId,
};
use teleprompt_scene::{BlockSource, BodyOrigin, Measured, SceneCompiler, ScenePlugins, Shot};
use teleprompt_script::config::Config;
use teleprompt_script::program::{ActionElement, Element, Program};
use teleprompt_voice::cache::CacheKey;
use teleprompt_voice::{SynthRequest, WordTiming};

use super::cue::at_offset_ms;
use super::{NarrationDetail, ShotSource, VoiceContext, PAUSE_SCENE};

/// An included file as an author would find it from where they invoked
/// teleprompt: `steps.mock`, not `./steps.mock`.
fn display_path(path: &Path) -> String {
    let s = path.display().to_string();
    s.strip_prefix("./").unwrap_or(&s).to_string()
}

/// A line waiting to pair with the first shot of the next action block.
struct Pending {
    input: NarrationInput,
    id: LineId,
    /// What a `cue=` on that block searches.
    text: String,
    config: Config,
}

/// How a block's shots become items, once the block has checked out.
struct Placing {
    plugin_name: String,
    policy: PolicyKind,
    align: Align,
    cue_ms: Option<u64>,
    origin: BodyOrigin,
}

/// Walks a program's elements into schedulable items, collecting every
/// diagnostic rather than stopping at the first.
///
/// A block that fails leaves any pending line unpaired, so the line pairs
/// with the next block that compiles, exactly as if the failed block were
/// not there.
pub(super) struct Walker<'a, 'v> {
    program: &'a Program,
    scenes: &'a ScenePlugins,
    voice: &'a VoiceContext<'v>,
    base_dir: &'a Path,
    pub(super) diags: Vec<Diagnostic>,
    pub(super) items: Vec<Item>,
    pub(super) narration: Vec<NarrationDetail>,
    pub(super) shots: BTreeMap<ShotId, ShotSource>,
    pub(super) warnings: Vec<String>,
    pending: Option<Pending>,
}

impl<'a, 'v> Walker<'a, 'v> {
    /// Walks every element of `program`, in order.
    pub(super) fn walk(
        program: &'a Program,
        scenes: &'a ScenePlugins,
        voice: &'a VoiceContext<'v>,
        base_dir: &'a Path,
    ) -> Self {
        let mut walker = Walker {
            program,
            scenes,
            voice,
            base_dir,
            diags: Vec::new(),
            items: Vec::new(),
            narration: Vec::new(),
            shots: BTreeMap::new(),
            warnings: Vec::new(),
            pending: None,
        };
        for element in &program.elements {
            match element {
                Element::Narration { .. } => walker.narration(element),
                Element::Action(a) => walker.action(a),
                Element::Pause { ms } => walker.pause(*ms),
            }
        }
        walker.flush();
        walker
    }

    /// A line no action block claimed becomes an item of its own.
    fn flush(&mut self) {
        if let Some(p) = self.pending.take() {
            self.items.push(Item {
                id: p.id.into(),
                narration: Some(p.input),
                action: None,
                policy: PolicyKind::Hold,
                align: Align::Start,
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
        let cache_key = teleprompt_voice::cache::key(backend, &backend_version, &req);
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
            name: config
                .voice
                .name
                .clone()
                .or_else(|| speaker.as_deref().map(teleprompt_script::ast::name_of)),
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

    fn action(&mut self, b: &ActionElement) {
        // A block `from` drafted runs commands lifted from another document;
        // warn until a human removes the attribute.
        if b.review.as_deref() == Some("pending") {
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
        let cue_ms = match b.cue.as_deref() {
            None => None,
            Some(phrase) => match self.cue_offset(phrase, b.policy) {
                Ok(ms) => ms,
                Err(d) => {
                    self.diags.push(d.at(b.span));
                    return;
                }
            },
        };
        let Some((plugin_name, plugin)) = self.plugin_for(&b.scene, &b.config) else {
            return;
        };
        let Some(shots) = self.split(plugin, b, body, &origin, fragment.as_deref()) else {
            return;
        };
        if shots.is_empty() {
            // An all-`mark` block has no shots. A line pairs only with the
            // block immediately after it, so flush it.
            self.flush();
            return;
        }
        let placing = Placing {
            plugin_name,
            policy: b.policy,
            align: b.align,
            cue_ms,
            origin,
        };
        self.push_shots(b, &shots, &placing);
    }

    /// The block's body, where its lines are numbered from, and the fragment
    /// an `include=file#fragment` names. `None` once the reason is reported.
    fn load_body(&mut self, b: &ActionElement) -> Option<(String, BodyOrigin, Option<String>)> {
        // The fragment is the plugin's to interpret (see `split`).
        let (include, fragment) = match b.include.as_deref().map(|i| i.split_once('#')) {
            Some(Some((path, frag))) => (Some(path), Some(frag.to_string())),
            _ => (b.include.as_deref(), None),
        };
        let Some(rel) = include else {
            return Some((
                b.body.to_string(),
                BodyOrigin::Inline { fence: b.span },
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
            // Scene compiler diagnostics then name this file.
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

    /// The plugin configured for `scene`, or its default.
    fn plugin_for(
        &mut self,
        scene: &str,
        config: &Config,
    ) -> Option<(String, &'a dyn SceneCompiler)> {
        let declared = config.scenes.get(scene);
        let plugin_name = declared.map_or_else(|| scene.to_string(), |s| s.plugin.clone());
        let scenes = self.scenes;
        let Some(plugin) = scenes.compiler(&plugin_name) else {
            let available = scenes.names().join(", ");
            if declared.is_none() && plugin_name == scene {
                self.diags.push(
                    Diagnostic::error(format!("unknown scene `{scene}`")).with_help(format!(
                        "use a scene plugin's name as the scene ({available}), or declare \
                             it under [scene.{scene}] in teleprompt.toml with its plugin"
                    )),
                );
                return None;
            }
            self.diags.push(
                Diagnostic::error(format!(
                    "scene `{scene}` needs the scene plugin `{plugin_name}`, which is not available"
                ))
                .with_help(format!("available scene plugins: {available}")),
            );
            return None;
        };
        Some((plugin_name, plugin))
    }

    /// Has the plugin validate the body, select its fragment, and split it
    /// into shots.
    fn split(
        &mut self,
        plugin: &dyn SceneCompiler,
        b: &ActionElement,
        body: String,
        origin: &BodyOrigin,
        fragment: Option<&str>,
    ) -> Option<Vec<Shot>> {
        let src = BlockSource {
            scene: b.scene.clone(),
            body,
            origin: origin.clone(),
        };
        let mut validated = match plugin.validate(&src) {
            Ok(v) => v,
            Err(mut e) => {
                self.diags.append(&mut e);
                return None;
            }
        };
        if let Some(fragment) = fragment {
            match plugin.select(&validated.body, fragment) {
                Ok(part) => validated.body = part,
                Err(why) => {
                    self.diags.push(Diagnostic::error(why).at(b.span));
                    return None;
                }
            }
        }
        match plugin.shots(&validated, &b.block_id) {
            Ok(s) => Some(s),
            Err(mut e) => {
                self.diags.append(&mut e);
                None
            }
        }
    }

    /// One item per shot; the first takes the pending line, if there is one.
    fn push_shots(&mut self, b: &ActionElement, shots: &[Shot], how: &Placing) {
        for (i, shot) in shots.iter().enumerate() {
            self.shots.insert(
                shot.id.clone(),
                ShotSource {
                    scene: b.scene.clone(),
                    plugin: how.plugin_name.clone(),
                    source: shot.source.clone(),
                    length: shot.length,
                },
            );
            let Some(measured) = self
                .stretched(b, how, shot.length)
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
                            how.plugin_name
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
                plugin: how.plugin_name.clone(),
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
                align: how.align,
                pacing: Pacing::from(&b.config),
            });
        }
    }

    /// A shot's length with the block's `stretch=` applied. Only a shot that
    /// states its own length can be stretched: the plugin re-times it to
    /// the new length ([`retime_stretched_shots`]). `None` once reported.
    fn stretched(
        &mut self,
        b: &ActionElement,
        how: &Placing,
        measured: Measured,
    ) -> Option<Measured> {
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
                .at(b.span)
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
                        how.plugin_name
                    ))
                    .at(b.span)
                    .with_help("it takes its line's length: lengthen the line instead"),
                );
                None
            }
        }
    }

    /// A `fit-line` shot's length: its own, or the block's `budget=` for
    /// one that states none. The picture leads, so it must have a length
    /// (docs/design.md#led-by-the-picture). `None` once reported.
    fn budgeted(
        &mut self,
        b: &ActionElement,
        how: &Placing,
        measured: Measured,
    ) -> Option<Measured> {
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
                        how.plugin_name
                    ))
                    .at(b.span)
                    .with_help("give the block the picture's length, e.g. `budget=6.5s`"),
                );
                None
            }
            (known, Some(_)) => {
                self.diags.push(
                    Diagnostic::error(format!(
                        "`budget` on a `{}` shot that states its own length ({} ms)",
                        how.plugin_name,
                        known.duration_ms().unwrap_or(0)
                    ))
                    .at(b.span)
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
                plugin: PAUSE_SCENE.into(),
                shot_hash: Hash::of(ms.to_string().as_bytes()),
                duration_ms: ms,
                duration_source: DurationSource::Exact,
                cue_ms: None,
                session: None,
            }),
            policy: PolicyKind::Hold,
            align: Align::Start,
            pacing: Pacing::from(&self.program.config),
        });
    }
}
