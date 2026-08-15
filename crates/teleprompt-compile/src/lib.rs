//! Walks a resolved [`Program`] and assembles it into a scheduled
//! [`Timeline`]: asks the scene registry to validate and split action
//! blocks, asks the voice backend for narration durations, pairs narration
//! with the action span that follows it into beats, and hands the beats to
//! the scheduler.
//!
//! This crate is the seam where `teleprompt-core`, `teleprompt-scene`,
//! `teleprompt-voice`, and `teleprompt-schedule` meet, so that they never
//! have to depend on one another.

use std::path::{Component, Path};

use teleprompt_core::config::default_adapter;
use teleprompt_core::program::{ChapterInfo, Item, Program};
use teleprompt_core::{Diagnostic, Diagnostics, Hash};
use teleprompt_scene::{BlockSource, BodyOrigin, Measured, SceneRegistry};
use teleprompt_schedule::{
    schedule, ActionInput, Beat, DurationSource, NarrationInput, Policy, Timeline,
};
use teleprompt_voice::{resolve_source, SynthRequest, VoiceBackend, VoiceSource, WordTiming};

pub mod manifest;
pub mod manifest_diff;

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
    pub word_timings: Option<Vec<WordTiming>>,
}

#[derive(Debug)]
pub struct CompileOutput {
    pub timeline: Timeline,
    pub warnings: Vec<String>,
    /// One entry per narration item that synthesized, in document order.
    pub narration: Vec<NarrationDetail>,
    /// The script's chapters, in document order. Carried here because
    /// `compile` drops the `Program` and `cmd::check::compile_script` —
    /// the CLI's only route into compilation — returns just this struct.
    /// Without it the manifest's chapter markers are unreachable from the
    /// command that has to write them.
    pub chapters: Vec<ChapterInfo>,
}

/// Compiles `program` into a scheduled [`Timeline`].
///
/// Pure given its inputs: the same program, registry, and voice backend
/// always produce byte-identical timeline JSON, since every backend
/// implementation this crate is compiled against (`NullVoice`, the mock
/// scene adapter) is itself deterministic.
pub fn compile(
    program: &Program,
    registry: &SceneRegistry,
    voice: &dyn VoiceBackend,
    base_dir: &Path,
    version: &str,
) -> Result<CompileOutput, Diagnostics> {
    let mut diags = Vec::new();
    let mut beats: Vec<Beat> = Vec::new();
    let mut narration_details: Vec<NarrationDetail> = Vec::new();

    // The narration waiting to be joined with the first span of the next
    // action block. `id` and `config` travel alongside it because they
    // belong to the beat, not to the `NarrationInput` payload itself.
    let mut pending: Option<NarrationInput> = None;
    let mut pending_id = String::new();
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
                    text: text.clone(),
                    locale: program.locale.clone(),
                    voice: config.voice.voice.clone(),
                    speed: config.voice.speed,
                };
                let synth = match voice.synthesize(&req) {
                    Ok(s) => s,
                    Err(e) => {
                        diags.push(Diagnostic::error(format!("segment `{id}`: {e}")));
                        continue;
                    }
                };

                narration_details.push(NarrationDetail {
                    segment_id: id.clone(),
                    text: text.clone(),
                    chapter: chapter.clone(),
                    chapter_index: *chapter_index,
                    synth_request: req,
                    word_timings: synth.word_timings.clone(),
                });

                pending = Some(NarrationInput {
                    segment_id: id.clone(),
                    source_hash: *source_hash,
                    audio_hash: synth.audio_hash,
                    duration_ms: synth.duration_ms,
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
                span,
            } => {
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

                let Some(parsed_policy) = Policy::parse(policy, align) else {
                    diags.push(
                        Diagnostic::error(format!("unknown policy `{policy}` or align `{align}`"))
                            .with_help(
                                "policy is hold|concurrent|stretch|trim; align is start|end|center",
                            ),
                    );
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

    let (timeline, warnings) = schedule(&beats, &program.script_name, &program.locale, version);
    Ok(CompileOutput {
        timeline,
        warnings,
        narration: narration_details,
        chapters: program.chapters.clone(),
    })
}
