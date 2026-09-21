//! Walks a resolved [`Program`] and assembles it into a scheduled
//! [`Timeline`]: asks the scene registry to validate and split action
//! blocks, takes each narration's duration from the synthesis cache when
//! the segment is already rendered and from a [`DurationEstimator`] when it
//! is not, pairs narration with the action span that follows it into beats,
//! and hands the beats to the scheduler.
//!
//! It never asks a voice backend for anything. It cannot: [`VoiceContext`]
//! offers no way to reach one, which is what keeps `check`, `plan`, and
//! `diff` synchronous and offline.
//!
//! This crate is the seam where `teleprompt-core`, `teleprompt-scene`,
//! `teleprompt-voice`, and `teleprompt-schedule` meet, so that they never
//! have to depend on one another.

use std::path::{Component, Path};

use teleprompt_cache::{CacheKey, VoiceCache};
use teleprompt_core::config::{default_adapter, OutputConfig};
use teleprompt_core::program::{ChapterInfo, Item, Program};
use teleprompt_core::voice::spoken;
use teleprompt_core::{Diagnostic, Diagnostics, Hash};
use teleprompt_scene::{BlockSource, BodyOrigin, Measured, SceneRegistry, Span};
use teleprompt_schedule::{
    schedule, ActionInput, Beat, DurationSource, NarrationInput, Policy, Timeline,
};
use teleprompt_voice::{resolve_source, DurationEstimator, SynthRequest, VoiceSource, WordTiming};

pub mod manifest;
pub mod manifest_diff;

/// Everything `compile` needs about voice — and deliberately not a backend.
///
/// `compile` runs on the inner loop: `check`, `plan`, and `diff` call it on
/// every run and must stay sub-second and offline. Passing a `VoiceBackend`
/// here is what would make that impossible, so the type simply does not
/// offer one. Audio is `dub`'s and `build`'s business.
pub struct VoiceContext<'a> {
    pub backend_id: &'a str,
    pub backend_version: &'a str,
    pub cache: &'a VoiceCache,
    pub estimator: &'a dyn DurationEstimator,
}

/// How an included file is spelled in a diagnostic: the way an author would
/// find it from where they invoked teleprompt, with a `./` prefix trimmed so
/// a script in the current directory yields `steps.mock`, not `./steps.mock`.
fn display_path(path: &Path) -> String {
    let s = path.display().to_string();
    s.strip_prefix("./").unwrap_or(&s).to_string()
}

/// What the `Timeline` deliberately does not carry. The timeline is a
/// review surface — a reader scanning a pacing diff does not want the
/// prose inlined in it, and word timings would dwarf everything else. The
/// narration manifest needs all of it, so `compile` keeps it here rather
/// than making a second pass to recover what it already had.
#[derive(Debug, Clone)]
pub struct NarrationDetail {
    pub segment_id: String,
    pub text: String,
    /// The chapter's slug, for publication. Two chapters with the same
    /// title share one, so this is display identity, not a join key.
    pub chapter: String,
    /// The chapter's position in `CompileOutput::chapters`. This is the
    /// join key: joining on the slug silently merged two identically-titled
    /// chapters into one marker and lost the second one's real start.
    pub chapter_index: usize,
    /// The exact request the duration in the timeline was measured from.
    ///
    /// Carried rather than reconstructed because a caller that needs audio
    /// (`dub`) must render from the *same* request `compile` measured. When
    /// `dub` built its own `SynthRequest` it dropped `voice` and `speed`, so
    /// a script with `voice: { speed: 2.0 }` published a 3250 ms duration
    /// alongside a 6500 ms file. One source, one request, no drift.
    pub synth_request: SynthRequest,
    /// The cache key `compile` looked up `synth_request` under. `dub` stores
    /// its render under this same key, so the two never drift apart.
    pub cache_key: CacheKey,
    pub word_timings: Option<Vec<WordTiming>>,
}

/// One action span's source, as the adapter split it.
///
/// The timeline identifies a span and says when it plays; it does not carry
/// what the span *is*. Anything that has to draw the scene — a renderer, a
/// preview — needs the source, and re-deriving it means re-running the
/// parse, the `include=` resolution and the adapter's own splitting, which
/// is this function's work done a second time and a second place for the
/// two to disagree.
///
/// Deliberately not in the published manifest: this is adapter-native code,
/// and a consumer drawing its own picture has no use for it.
#[derive(Debug, Clone)]
pub struct SpanSource {
    pub id: String,
    pub scene: String,
    pub adapter: String,
    pub source: String,
}

#[derive(Debug)]
pub struct CompileOutput {
    pub timeline: Timeline,
    pub warnings: Vec<String>,
    /// One entry per narration item that synthesized, in document order.
    pub narration: Vec<NarrationDetail>,
    /// Every action span's source, in document order.
    pub spans: Vec<SpanSource>,
    /// The script's chapters, in document order. Carried here because
    /// `compile` drops the `Program` and `cmd::check::compile_script` —
    /// the CLI's only route into compilation — returns just this struct.
    /// Without it the manifest's chapter markers are unreachable from the
    /// command that has to write them.
    pub chapters: Vec<ChapterInfo>,
    /// The frame the script asked for, resolved through every layer.
    ///
    /// Here for the same reason `chapters` is: `compile` drops the
    /// `Program`, and a render has no other route to the script's own
    /// `output:` block. It is deliberately not in the narration manifest —
    /// that publishes when things are spoken, and the size of the picture
    /// is not a timing fact.
    pub output: OutputConfig,
}

/// Where in a narration a cued action should start.
///
/// With no word timings from the backend — which is every backend today —
/// the offset is interpolated from where the phrase sits in the sentence.
/// That is an approximation and says so: speech is not uniform, and a long
/// word takes longer than a short one. It lands within a syllable or two on
/// a sentence, which is the difference between typing a command while it is
/// being named and typing it half a paragraph early. When a backend does
/// publish word timings, this is the one place that has to change.
fn cue_offset_ms(
    phrase: &str,
    text: &str,
    narration: Option<&NarrationInput>,
    policy: &str,
) -> Result<Option<u64>, Diagnostic> {
    if policy != "concurrent" {
        return Err(Diagnostic::error(format!(
            "`at=\"{phrase}\"` needs `policy=concurrent`, not `{policy}`"
        ))
        .with_help(
            "hold runs the action after the narration and the stretch policies \
             size it to fit; a cue only means something where the two run together",
        ));
    }
    let Some(narration) = narration else {
        return Err(
            Diagnostic::error(format!("`at=\"{phrase}\"` has no narration to start in"))
                .with_help("a cue names a phrase in the paragraph above the block"),
        );
    };

    let Some(at) = text.find(phrase) else {
        return Err(Diagnostic::error(format!(
            "`at=\"{phrase}\"` is not in the narration above it"
        ))
        .with_help(format!("the paragraph reads: {text}")));
    };

    if text.is_empty() {
        return Ok(None);
    }
    let fraction = at as f64 / text.chars().count() as f64;
    Ok(Some(
        (narration.duration_ms as f64 * fraction).round() as u64
    ))
}

/// Puts the scheduler's decision back into the adapter's own language.
///
/// `stretch-action` and `trim-action` change how long an action should
/// take. Until this pass, that was a number on a timeline and nothing else:
/// a capture would run the tape at its authored pace and whatever remained
/// of the slot would be a held frame. Here the adapter re-writes the span
/// to last exactly as long as it was scheduled for, and what is published
/// is that tape.
///
/// The span's hash moves with it, which is the point rather than a side
/// effect: spec §5.1 puts slot duration in the video cache key, and hashing
/// the tape that will actually be captured does that exactly. An adapter
/// that cannot re-time says so, the source stands, and the renderer holds
/// the last frame for the difference.
fn retime_stretched_spans(
    timeline: &mut Timeline,
    spans: &mut [SpanSource],
    registry: &SceneRegistry,
) {
    for entry in &mut timeline.entries {
        let Some(action) = entry.action.as_mut() else {
            continue;
        };
        let Some(published) = spans.iter_mut().find(|s| s.id == action.span) else {
            continue;
        };
        let Some(adapter) = registry.get(&action.adapter) else {
            continue;
        };

        // Nothing to do where the schedule took the adapter's own number,
        // which is every policy but the two that change it.
        let span = Span {
            id: published.id.clone(),
            source: published.source.clone(),
            hash: action.span_hash,
            index: 0,
        };
        if adapter.estimate(&span).duration_ms() == Some(action.duration_ms) {
            continue;
        }

        if let Some(source) = adapter.retime(&span, action.duration_ms) {
            action.span_hash = Hash::of(source.as_bytes());
            action.duration_source = "exact".to_string();
            published.source = source;
        }
    }
}

/// Compiles `program` into a scheduled [`Timeline`].
///
/// Pure given its inputs: the same program, registry, and voice context
/// always produce byte-identical timeline JSON. It never synthesizes —
/// durations come from `voice_ctx.cache` on a hit and `voice_ctx.estimator`
/// on a miss — which is what keeps this on the sub-second, offline path
/// `check`, `plan`, and `diff` run on every edit.
pub fn compile(
    program: &Program,
    registry: &SceneRegistry,
    voice_ctx: &VoiceContext,
    base_dir: &Path,
    version: &str,
) -> Result<CompileOutput, Diagnostics> {
    let mut diags = Vec::new();
    let mut beats: Vec<Beat> = Vec::new();
    let mut narration_details: Vec<NarrationDetail> = Vec::new();
    let mut span_sources: Vec<SpanSource> = Vec::new();
    let mut cache_warnings: Vec<String> = Vec::new();

    // The narration waiting to be joined with the first span of the next
    // action block. `id` and `config` travel alongside it because they
    // belong to the beat, not to the `NarrationInput` payload itself.
    let mut pending: Option<NarrationInput> = None;
    let mut pending_id = String::new();
    // The words of the narration waiting for an action block, kept so an
    // `at="…"` cue can be found in them.
    let mut pending_text = String::new();
    let mut pending_config = program.config.clone();

    let flush = |beats: &mut Vec<Beat>,
                 pending: &mut Option<NarrationInput>,
                 id: &mut String,
                 cfg: &teleprompt_core::config::Config| {
        if let Some(n) = pending.take() {
            beats.push(Beat {
                id: std::mem::take(id),
                narration: Some(n),
                action: None,
                policy: Policy::Hold,
                config: cfg.clone(),
            });
        }
    };

    for item in &program.items {
        match item {
            Item::Narration {
                id,
                text,
                source_hash,
                chapter,
                chapter_index,
                config,
                ..
            } => {
                // An action-less narration (no action block follows before
                // the next narration, pause, or end of program) becomes its
                // own beat; flush whatever was pending first.
                flush(&mut beats, &mut pending, &mut pending_id, &pending_config);

                let Some(requested) = VoiceSource::parse(&config.voice.source) else {
                    diags.push(
                        Diagnostic::error(format!(
                            "unknown voice source `{}`",
                            config.voice.source
                        ))
                        .with_help("voice.source is recorded|cloned|synthetic"),
                    );
                    continue;
                };
                let resolution = match resolve_source(requested, &|tier| match tier {
                    VoiceSource::Recorded => Err("no takes recorded (M0 has no recorder)".into()),
                    VoiceSource::Cloned => Err("no voice profile enrolled".into()),
                    VoiceSource::Synthetic => Ok(()),
                }) {
                    Ok(r) => r,
                    Err(e) => {
                        diags.push(Diagnostic::error(e));
                        continue;
                    }
                };

                let req = SynthRequest {
                    // The voice is given the pronunciation; everything
                    // published keeps the spelling. The mapped text is in
                    // the cache key by construction, so correcting how a
                    // word is said re-renders the audio that said it wrong.
                    text: spoken(text, &config.voice.pronounce),
                    locale: program.locale.clone(),
                    voice: config.voice.voice.clone(),
                    speed: config.voice.speed,
                };
                let cache_key =
                    teleprompt_cache::key(voice_ctx.backend_id, voice_ctx.backend_version, &req);

                // Metadata only: `compile` never touches the audio itself,
                // and reading the WAV back on every warm hit only to drop it
                // would put the whole cache's audio through `plan` on every
                // run.
                let read = match voice_ctx.cache.lookup_meta(&cache_key) {
                    Ok(c) => c,
                    Err(e) => {
                        diags.push(Diagnostic::error(format!("segment `{id}`: {e}")));
                        continue;
                    }
                };
                // An unreadable entry is a miss, not an error — the cache is
                // derived and self-healing. It is still worth saying out
                // loud, because otherwise a segment silently reverts from
                // `measured` to `estimated` with no explanation.
                if let Some(w) = read.warning() {
                    cache_warnings.push(format!("segment `{id}`: {w}"));
                }
                let (duration_ms, duration_source, word_timings) = match read.hit() {
                    Some(hit) => (hit.duration_ms, DurationSource::Measured, hit.word_timings),
                    None => (
                        voice_ctx.estimator.estimate_ms(&req),
                        DurationSource::Estimated,
                        None,
                    ),
                };

                narration_details.push(NarrationDetail {
                    segment_id: id.clone(),
                    text: text.clone(),
                    chapter: chapter.clone(),
                    chapter_index: *chapter_index,
                    synth_request: req,
                    cache_key: cache_key.clone(),
                    word_timings,
                });

                pending = Some(NarrationInput {
                    segment_id: id.clone(),
                    source_hash: *source_hash,
                    // The cache key, not a synthesis result: this path never
                    // synthesizes. It identifies the audio this segment
                    // resolves to, so it moves exactly when the audio would.
                    // The manifest's `audio_hash` is a different thing — a
                    // hash of the bytes `dub` actually wrote.
                    audio_hash: Hash::of(cache_key.to_string().as_bytes()),
                    duration_ms,
                    duration_source,
                    // Read off the *narration item's* own resolved config,
                    // which is the only place a segment-level `lead_in=` /
                    // `tail=` survives. The beat this narration ends up in
                    // may carry the following action block's config instead.
                    lead_in_ms: config.timing.lead_in_ms,
                    tail_ms: config.timing.tail_ms,
                    voice_source: resolution.requested,
                    voice_source_actual: resolution.actual,
                    downgrade_reason: resolution.downgrade_reason,
                });
                pending_id = id.clone();
                pending_text = text.clone();
                pending_config = config.clone();
            }

            Item::Action {
                block_id,
                scene,
                body,
                include,
                config,
                policy,
                align,
                cue,
                review,
                span,
            } => {
                // A tape `from` generated types a command lifted out of
                // someone else's document. `check` says so on every run
                // until a human has read it and removed the attribute —
                // the draft is a draft until somebody says otherwise.
                if review.as_deref() == Some("pending") {
                    cache_warnings.push(format!(
                        "action block `{block_id}` is marked `review=pending`: \
it was drafted from another document and has not been reviewed. \
Read it, then remove the attribute."
                    ));
                }
                // Controller ruling F5: an `include=` path is inspected
                // *before* anything is joined to `base_dir`. `Path::join`
                // followed by `starts_with` never rejects `..` — `..` is
                // just another component, so the joined path's prefix is
                // always `base_dir`'s components regardless of how many
                // `..` follow. Checking the raw components (and rejecting an
                // absolute path outright) is what actually stops traversal;
                // canonicalising first would follow symlinks out of the
                // project and let a malicious symlink pass the check.
                let (body, origin) = match include {
                    Some(rel) => {
                        let p = Path::new(rel);
                        if p.is_absolute()
                            || p.components().any(|c| matches!(c, Component::ParentDir))
                        {
                            diags.push(Diagnostic::error(format!(
                                "included file `{rel}` resolves outside the project"
                            )));
                            continue;
                        }
                        if !body.trim().is_empty() {
                            diags.push(Diagnostic::error(format!(
                                "action block has both a body and an `include`: `{rel}`"
                            )));
                            continue;
                        }
                        let path = base_dir.join(rel);
                        match std::fs::read_to_string(&path) {
                            Ok(s) => (
                                s,
                                // Diagnostics about this body name the
                                // included file, spelled the way an author
                                // would find it from where they invoked
                                // teleprompt — `scripts/steps.mock`, not the
                                // script's own path with an invented line.
                                BodyOrigin::Included {
                                    path: display_path(&path),
                                },
                            ),
                            Err(e) => {
                                // Name the attribute as the author wrote it
                                // *and* where it resolved to, when those
                                // differ — the first is what they search for,
                                // the second is what actually went missing.
                                let resolved = display_path(&path);
                                let at = if resolved == *rel {
                                    String::new()
                                } else {
                                    format!(" ({resolved})")
                                };
                                diags.push(Diagnostic::error(format!(
                                    "cannot read included file `{rel}`{at}: {e}"
                                )));
                                continue;
                            }
                        }
                    }
                    None => (body.clone(), BodyOrigin::Inline { fence: *span }),
                };

                let cue_ms = match cue {
                    None => None,
                    Some(phrase) => {
                        match cue_offset_ms(phrase, &pending_text, pending.as_ref(), policy) {
                            Ok(ms) => ms,
                            Err(d) => {
                                diags.push(d.at(*span));
                                continue;
                            }
                        }
                    }
                };

                let Some(parsed_policy) = Policy::parse(policy, align) else {
                    // A policy renamed since the author last wrote a script
                    // gets the new spelling rather than the generic list —
                    // "unknown policy `trim`" is a puzzle when `trim-action`
                    // is sitting right there.
                    let d = match Policy::renamed_hint(policy) {
                        Some(current) => Diagnostic::error(format!(
                            "policy `{policy}` was renamed to `{current}`"
                        ))
                        .with_help(format!(
                            "write `policy={current}`; it adjusts the action, never the narration"
                        )),
                        None => Diagnostic::error(format!(
                            "unknown policy `{policy}` or align `{align}`"
                        ))
                        .with_help(
                            "policy is hold|concurrent|stretch-action|trim-action; \
                             align is start|end|center",
                        ),
                    };
                    diags.push(d);
                    continue;
                };

                let adapter_name = config
                    .scenes
                    .get(scene)
                    .map(|s| s.adapter.clone())
                    .unwrap_or_else(|| default_adapter(scene).to_string());

                let Some(adapter) = registry.get(&adapter_name) else {
                    diags.push(
                        Diagnostic::error(format!(
                            "scene `{scene}` needs adapter `{adapter_name}`, but no adapter `{adapter_name}` is available"
                        ))
                        .with_help(format!("available adapters: {}", registry.available().join(", "))),
                    );
                    continue;
                };

                // Controller ruling F12 (second half): pass the body's real
                // origin so adapter diagnostics point at the true file and
                // line, not a fabricated `line: 0` and not the script's own
                // path when the body came from somewhere else.
                let src = BlockSource {
                    scene: scene.clone(),
                    body: body.clone(),
                    origin: origin.clone(),
                };
                let validated = match adapter.validate(&src) {
                    Ok(v) => v,
                    Err(mut e) => {
                        diags.append(&mut e);
                        continue;
                    }
                };
                let spans = match adapter.spans(&validated, block_id) {
                    Ok(s) => s,
                    Err(mut e) => {
                        diags.append(&mut e);
                        continue;
                    }
                };

                if spans.is_empty() {
                    // Task 6's F13 filter can reduce an all-`mark` block to
                    // zero surviving spans. Pairing stays scoped to the
                    // *immediately* following action item, so a pending
                    // narration must be flushed as its own beat here rather
                    // than left to be picked up by a later action block.
                    flush(&mut beats, &mut pending, &mut pending_id, &pending_config);
                    continue;
                }

                for (i, span) in spans.iter().enumerate() {
                    span_sources.push(SpanSource {
                        id: span.id.clone(),
                        scene: scene.clone(),
                        adapter: adapter_name.clone(),
                        source: span.source.clone(),
                    });
                    let measured = adapter.estimate(span);
                    let action = ActionInput {
                        span_id: span.id.clone(),
                        scene: scene.clone(),
                        adapter: adapter_name.clone(),
                        span_hash: span.hash,
                        duration_ms: measured.duration_ms().unwrap_or(0),
                        duration_source: match measured {
                            Measured::Exact(_) => DurationSource::Exact,
                            Measured::Estimated(_) => DurationSource::Estimated,
                            Measured::Unknown => DurationSource::Estimated,
                        },
                        // Only the span paired with the narration can be
                        // cued to a word in it; the rest follow it.
                        cue_ms: if i == 0 { cue_ms } else { None },
                    };

                    if i == 0 && pending.is_some() {
                        beats.push(Beat {
                            id: std::mem::take(&mut pending_id),
                            narration: pending.take(),
                            action: Some(action),
                            policy: parsed_policy,
                            config: config.clone(),
                        });
                    } else {
                        beats.push(Beat {
                            id: span.id.clone(),
                            narration: None,
                            action: Some(action),
                            policy: parsed_policy,
                            config: config.clone(),
                        });
                    }
                }
            }

            Item::Pause { ms } => {
                flush(&mut beats, &mut pending, &mut pending_id, &pending_config);
                let id = format!("pause-{}", beats.len());
                beats.push(Beat {
                    id: id.clone(),
                    narration: None,
                    // A pause carries its duration in the action slot, tagged
                    // `scene: "pause"`, so the scheduler's existing hold
                    // arithmetic applies unchanged with no third case.
                    action: Some(ActionInput {
                        span_id: id,
                        scene: "pause".into(),
                        adapter: "pause".into(),
                        span_hash: Hash::of(ms.to_string().as_bytes()),
                        duration_ms: *ms,
                        duration_source: DurationSource::Exact,
                        cue_ms: None,
                    }),
                    policy: Policy::Hold,
                    config: program.config.clone(),
                });
            }
        }
    }

    flush(&mut beats, &mut pending, &mut pending_id, &pending_config);

    let d = Diagnostics(diags);
    if d.has_errors() {
        return Err(d);
    }

    let (mut timeline, scheduling_warnings) =
        schedule(&beats, &program.script_name, &program.locale, version);
    retime_stretched_spans(&mut timeline, &mut span_sources, registry);
    // Cache warnings first: they explain why the numbers the scheduler then
    // warns about are what they are.
    let mut warnings = cache_warnings;
    warnings.extend(scheduling_warnings);
    Ok(CompileOutput {
        timeline,
        warnings,
        narration: narration_details,
        spans: span_sources,
        chapters: program.chapters.clone(),
        output: program.config.output.clone(),
    })
}
