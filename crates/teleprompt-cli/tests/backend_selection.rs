//! Does the CLI actually use the backend the script selected?
//!
//! Spec §4.1 claims adding a backend is "one line there and one new crate —
//! nothing else in the workspace changes". A workspace with exactly one
//! backend registered cannot check that claim: `resolve` rejects every other
//! id, so a `dub` that ignored the resolution entirely would look identical.
//! These tests register a backend this build does not ship and follow it
//! through key, synthesis, cache, and manifest.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use teleprompt_cli::project::Project;
use teleprompt_voice::{
    DurationEstimator, LanguageSupport, Pcm, SynthRequest, Synthesized, VoiceBackend,
    VoiceCapabilities, VoiceError, VoiceRegistry,
};
use teleprompt_voice_null::{NullVoice, WpmEstimator};

/// A backend that is not `null`: a different id, a different version, a
/// different sample rate, and audible samples rather than silence.
///
/// Its length deliberately matches `WpmEstimator`'s prediction exactly, so
/// nothing here depends on the length guard — what these tests are about is
/// *which* backend produced the bytes, not how long they are.
struct ToneVoice;

const TONE_SAMPLE_RATE: u32 = 24_000;

#[async_trait]
impl VoiceBackend for ToneVoice {
    fn id(&self) -> &str {
        "tone"
    }

    fn capabilities(&self) -> VoiceCapabilities {
        VoiceCapabilities {
            languages: LanguageSupport::Any,
            cloning: false,
            cross_lingual: false,
            word_timings: false,
            ssml: false,
            speed_control: true,
            version: "tone-9.9.9".to_string(),
        }
    }

    async fn synthesize(&self, req: &SynthRequest) -> Result<Synthesized, VoiceError> {
        let ms = WpmEstimator::default().estimate_ms(req);
        let frames = (ms * TONE_SAMPLE_RATE as u64 / 1000) as usize;
        Ok(Synthesized {
            pcm: Pcm {
                sample_rate: TONE_SAMPLE_RATE,
                channels: 1,
                // Not silence. This is the whole discriminator: `NullVoice`
                // can only ever produce zeroes.
                samples: vec![4242; frames],
            },
            word_timings: None,
        })
    }
}

fn registry_with_tone() -> VoiceRegistry {
    let mut r = VoiceRegistry::default();
    r.register(Arc::new(NullVoice::default()));
    r.register(Arc::new(ToneVoice));
    r
}

fn tempdir(tag: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "teleprompt-backend-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

const SCRIPT: &str = "\
---
voice:
  backend: tone
---

# Quick start

Every video in this repository is built from a script you can read.
";

fn project_with(tag: &str, script: &str) -> (Project, PathBuf) {
    let dir = tempdir(tag);
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    std::fs::write(dir.join("scripts/test.md"), script).unwrap();
    let project = Project::discover(&dir).unwrap();
    let path = dir.join("scripts/test.md");
    (project, path)
}

/// The 16-bit samples in a WAV's `data` chunk.
fn samples(wav: &[u8]) -> Vec<i16> {
    assert_eq!(&wav[0..4], b"RIFF");
    let len = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
    wav[44..44 + len]
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect()
}

/// I2. `run_dub` built a `NullVoice` unconditionally while `cache_key` came
/// from the *resolved* backend's `id()` and `capabilities().version`. With
/// only `null` registered the two coincided; with a second backend the
/// script asks for kokoro and gets silence — written into the
/// content-addressed cache under kokoro's key, permanent, and reported as
/// `measured` by every later `plan`.
#[tokio::test]
async fn dub_synthesizes_with_the_backend_the_script_resolved_to() {
    let (project, script) = project_with("resolved", SCRIPT);
    let out_root = project.root.join("public/narration");

    let result = teleprompt_cli::cmd::dub::run_dub_with(
        &registry_with_tone(),
        &project,
        &script,
        "en",
        &out_root,
        false,
    )
    .await
    .unwrap_or_else(|e| match e {
        teleprompt_cli::cmd::dub::DubError::Validation(v) => panic!("validation: {v:?}"),
        teleprompt_cli::cmd::dub::DubError::Runtime(r) => panic!("runtime: {r}"),
    });

    assert_eq!(result.manifest.segments.len(), 1);
    let wav = std::fs::read(out_root.join("en").join(&result.manifest.segments[0].audio)).unwrap();

    let s = samples(&wav);
    assert!(!s.is_empty(), "no audio was written at all");
    assert!(
        s.iter().all(|&v| v == 4242),
        "`dub` published audio the resolved backend did not produce — \
         `NullVoice` silence stored under `tone`'s cache key"
    );
}

/// The same finding's second half: `AudioInfo` was seeded from
/// `NULL_SAMPLE_RATE`, so the manifest described every backend's audio with
/// the null backend's rate. A consumer configures its player from this once.
#[tokio::test]
async fn the_manifest_reports_the_rate_the_backend_actually_produced() {
    let (project, script) = project_with("rate", SCRIPT);
    let out_root = project.root.join("public/narration");

    let result = teleprompt_cli::cmd::dub::run_dub_with(
        &registry_with_tone(),
        &project,
        &script,
        "en",
        &out_root,
        false,
    )
    .await
    .ok()
    .expect("dub must succeed");

    assert_eq!(
        result.manifest.audio.sample_rate, TONE_SAMPLE_RATE,
        "the manifest must describe the audio beside it, not the null backend's rate"
    );
    assert_eq!(result.manifest.audio.channels, 1);
}

/// And `check` still refuses a backend nothing registers — the seam is for
/// tests, not a way for a script to name anything it likes.
#[test]
fn an_unregistered_backend_is_still_rejected() {
    let (project, script) = project_with("unknown", SCRIPT);
    let Err(errors) = teleprompt_cli::cmd::check::compile_script(&project, &script, "en") else {
        panic!("the default registry does not ship `tone`, so this must fail");
    };
    assert!(errors.join("\n").contains("tone"), "{errors:?}");
}
