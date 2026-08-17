//! Does the CLI actually use the backend the script selected?
//!
//! Spec §4.1 claims adding a backend is "one line there and one new crate —
//! nothing else in the workspace changes". A workspace with exactly one
//! backend registered cannot check that claim: `resolve` rejects every other
//! id, so a `dub` that ignored the resolution entirely would look identical.
//! These tests register a backend this build does not ship and follow it
//! through key, synthesis, cache, and manifest.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use teleprompt_cli::project::Project;
use teleprompt_voice::async_trait;
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
            speed: Some(0.25..=4.0),
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

/// A backend that renders *longer* than the word-count estimate predicts —
/// 120 wpm against `WpmEstimator`'s 150 — which is the normal condition for
/// any real backend and the one `null` uniquely does not exhibit.
struct DrawlVoice;

const DRAWL_WPM: f64 = 120.0;

#[async_trait]
impl VoiceBackend for DrawlVoice {
    fn id(&self) -> &str {
        "drawl"
    }

    fn capabilities(&self) -> VoiceCapabilities {
        VoiceCapabilities {
            languages: LanguageSupport::Any,
            cloning: false,
            cross_lingual: false,
            word_timings: false,
            ssml: false,
            speed: Some(0.25..=4.0),
            version: "drawl-1".to_string(),
        }
    }

    async fn synthesize(&self, req: &SynthRequest) -> Result<Synthesized, VoiceError> {
        let ms = teleprompt_voice_null::estimator::estimate_ms(&req.text, DRAWL_WPM, req.speed);
        let frames = (ms * TONE_SAMPLE_RATE as u64 / 1000) as usize;
        Ok(Synthesized {
            pcm: Pcm {
                sample_rate: TONE_SAMPLE_RATE,
                channels: 1,
                samples: vec![7; frames],
            },
            word_timings: None,
        })
    }
}

fn registry_with_tone() -> teleprompt_cli::voice::Backends {
    let mut r = VoiceRegistry::default();
    r.register(Arc::new(NullVoice::default()));
    r.register(Arc::new(ToneVoice));
    r.register(Arc::new(DrawlVoice));
    teleprompt_cli::voice::Backends::from_registry(r)
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

const DRAWL_SCRIPT: &str = "\
---
voice:
  backend: drawl
---

# Quick start

Every video in this repository is built from a script you can read.
";

/// I1. The WAV-length guard ran inside the render loop, against the
/// *pre-render* compile's timeline — which on a cold cache holds estimates.
/// The value the manifest actually publishes comes from the recompile that
/// follows the loop.
///
/// With `null` the two always agreed, because `NullVoice::synthesize` and
/// `WpmEstimator::estimate_ms` call the same function. For any backend whose
/// render differs from the word-count estimate the first `dub` of every new
/// segment failed, quoting a duration the manifest would never have
/// published, and the identical second run succeeded off the now-warm cache.
#[tokio::test]
async fn a_backend_that_renders_longer_than_the_estimate_dubs_on_the_first_run() {
    let (project, script) = project_with("drawl", DRAWL_SCRIPT);
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
        teleprompt_cli::cmd::dub::DubError::Runtime(r) => {
            panic!("the first dub of a cold segment must not fail: {r}")
        }
    });

    let seg = &result.manifest.segments[0];
    let wav = std::fs::read(out_root.join("en").join(&seg.audio)).unwrap();
    let rendered_ms = samples(&wav).len() as u64 * 1000 / TONE_SAMPLE_RATE as u64;

    // The disagreement is real, so this is not passing because the fixture
    // happens to render at exactly the estimated length.
    let estimated_ms = WpmEstimator::default().estimate_ms(&SynthRequest {
        text: seg.text.clone(),
        locale: "en".to_string(),
        voice: None,
        speed: 1.0,
    });
    assert_ne!(
        rendered_ms, estimated_ms,
        "the fixture must exercise a backend that disagrees with the estimate"
    );

    assert_eq!(
        seg.duration_ms, rendered_ms,
        "the manifest must publish the length of the file beside it"
    );
    assert_eq!(
        seg.duration_source, "measured",
        "the recompile ran against the warm cache, so this is a measurement"
    );
}

/// Fails with `Transient` the first two times, then succeeds — the shape of
/// a server having a brief bad moment mid-run.
struct FlakyVoice {
    calls: AtomicUsize,
    kind: teleprompt_voice::ErrorKind,
    fail_times: usize,
}

#[async_trait]
impl VoiceBackend for FlakyVoice {
    fn id(&self) -> &str {
        "flaky"
    }

    fn capabilities(&self) -> VoiceCapabilities {
        VoiceCapabilities {
            languages: LanguageSupport::Any,
            cloning: false,
            cross_lingual: false,
            word_timings: false,
            ssml: false,
            speed: Some(0.25..=4.0),
            version: "flaky-1".to_string(),
        }
    }

    async fn synthesize(&self, req: &SynthRequest) -> Result<Synthesized, VoiceError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if n < self.fail_times {
            return Err(VoiceError::new("flaky", self.kind, "not yet"));
        }
        let ms = WpmEstimator::default().estimate_ms(req);
        let frames = (ms * TONE_SAMPLE_RATE as u64 / 1000) as usize;
        Ok(Synthesized {
            pcm: Pcm {
                sample_rate: TONE_SAMPLE_RATE,
                channels: 1,
                samples: vec![11; frames],
            },
            word_timings: None,
        })
    }
}

const FLAKY_SCRIPT: &str = "\
---
voice:
  backend: flaky
---

# Quick start

Every video in this repository is built from a script you can read.
";

async fn dub_against_flaky(
    tag: &str,
    kind: teleprompt_voice::ErrorKind,
    fail_times: usize,
) -> (
    Result<teleprompt_cli::cmd::dub::DubOutput, teleprompt_cli::cmd::dub::DubError>,
    usize,
) {
    let backend = Arc::new(FlakyVoice {
        calls: AtomicUsize::new(0),
        kind,
        fail_times,
    });
    let mut r = VoiceRegistry::default();
    r.register(Arc::new(NullVoice::default()));
    r.register(backend.clone());
    let backends = teleprompt_cli::voice::Backends::from_registry(r);

    let (project, script) = project_with(tag, FLAKY_SCRIPT);
    let out_root = project.root.join("public/narration");
    let result = teleprompt_cli::cmd::dub::run_dub_with(
        &backends, &project, &script, "en", &out_root, false,
    )
    .await;
    let calls = backend.calls.load(Ordering::SeqCst);
    (result, calls)
}

/// A transient failure mid-run used to lose the whole dub — and with a paid
/// backend, everything already synthesized before it.
#[tokio::test(start_paused = true)]
async fn dub_survives_a_transient_failure() {
    let (result, calls) =
        dub_against_flaky("flaky-transient", teleprompt_voice::ErrorKind::Transient, 2).await;
    if let Err(e) = result {
        match e {
            teleprompt_cli::cmd::dub::DubError::Validation(v) => panic!("validation: {v:?}"),
            teleprompt_cli::cmd::dub::DubError::Runtime(r) => panic!("runtime: {r}"),
        }
    }
    assert_eq!(calls, 3, "two failures then a success");
}

/// The other half, and the reason the taxonomy exists: retrying a spent
/// allowance just restates it four times more slowly.
#[tokio::test(start_paused = true)]
async fn dub_does_not_retry_a_fatal_failure() {
    let (result, calls) =
        dub_against_flaky("flaky-quota", teleprompt_voice::ErrorKind::Quota, 99).await;
    assert!(result.is_err());
    assert_eq!(calls, 1, "Quota must not be retried");
}

/// A backend that never recovers still fails the run, after a bounded
/// number of attempts rather than forever.
#[tokio::test(start_paused = true)]
async fn dub_gives_up_on_a_permanently_transient_backend() {
    let (result, calls) =
        dub_against_flaky("flaky-forever", teleprompt_voice::ErrorKind::Transient, 99).await;
    assert!(result.is_err());
    assert_eq!(calls, 4, "RetryPolicy::default().max_attempts");
}
