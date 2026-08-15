//! Walks a resolved [`Program`] and assembles it into a scheduled
//! [`Timeline`]: asks the scene registry to validate and split action
//! blocks, asks the voice backend for narration durations, pairs narration
//! with the action span that follows it into beats, and hands the beats to
//! the scheduler.
//!
//! This crate is the seam where `teleprompt-core`, `teleprompt-scene`,
//! `teleprompt-voice`, and `teleprompt-schedule` meet, so that they never
//! have to depend on one another.

use teleprompt_core::config::default_adapter;
use teleprompt_core::program::{Item, Program};
use teleprompt_core::{Diagnostic, Diagnostics, Hash};
use teleprompt_scene::{BlockSource, Measured, SceneRegistry};
use teleprompt_schedule::{
    schedule, ActionInput, Beat, DurationSource, NarrationInput, Policy, Timeline,
};
use teleprompt_voice::{resolve_source, SynthRequest, VoiceBackend, VoiceSource};

#[derive(Debug)]
pub struct CompileOutput {
    pub timeline: Timeline,
    pub warnings: Vec<String>,
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
    version: &str,
) -> Result<CompileOutput, Diagnostics> {
    let mut diags = Vec::new();
    let mut beats: Vec<Beat> = Vec::new();

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
                config,
                ..
            } => {
                // An action-less narration (no action block follows before
                // the next narration, pause, or end of program) becomes its
                // own beat; flush whatever was pending first.
                flush(&mut beats, &mut pending, &mut pending_id, &pending_config);

                let requested =
                    VoiceSource::parse(&config.voice.source).unwrap_or(VoiceSource::Synthetic);
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

                pending = Some(NarrationInput {
                    segment_id: id.clone(),
                    source_hash: *source_hash,
                    audio_hash: synth.audio_hash,
                    duration_ms: synth.duration_ms,
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
                config,
                policy,
                align,
                span,
            } => {
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

                // Controller ruling F12 (second half): pass the action
                // item's real span so adapter diagnostics point at the true
                // source line, not a fabricated `line: 0`.
                let src = BlockSource {
                    scene: scene.clone(),
                    body: body.clone(),
                    span: *span,
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
    Ok(CompileOutput { timeline, warnings })
}
