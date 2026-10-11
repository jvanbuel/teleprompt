//! `Lines` against an in-process voice: where each line's audio comes from.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use teleprompt_core::{LineId, Silent, SourceSpan};
use teleprompt_pipeline::compile::NarrationDetail;
use teleprompt_project::voicing::Lines;
use teleprompt_project::Failure;
use teleprompt_voice::backends::Backends;
use teleprompt_voice::cache::VoiceCache;
use teleprompt_voice::takes::Takes;
use teleprompt_voice::VoiceRegistry;
use teleprompt_voice::{async_trait, Pcm, SynthRequest, Synthesized, VoiceBackend, VoiceError};

/// A voice that counts its requests and speaks a tenth of a second of
/// silence at 24 kHz, offering only the voice `alice`.
#[derive(Default)]
struct Counting {
    calls: AtomicUsize,
}

#[async_trait]
impl VoiceBackend for Counting {
    fn id(&self) -> &str {
        "fake"
    }
    fn version(&self) -> String {
        "1".into()
    }
    async fn synthesize(&self, _req: &SynthRequest) -> Result<Synthesized, VoiceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Synthesized {
            pcm: Pcm {
                sample_rate: 24_000,
                channels: 1,
                samples: vec![0; 2_400],
            },
            word_timings: None,
        })
    }
    async fn voices(&self) -> Option<Result<Vec<String>, VoiceError>> {
        Some(Ok(vec!["alice".into()]))
    }
}

struct Fixture {
    _dir: teleprompt_testkit::TestDir,
    voice: Arc<Counting>,
    takes_dir: std::path::PathBuf,
    cache_dir: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = teleprompt_testkit::test_dir("voicing");
        let (takes_dir, cache_dir) = (dir.path().join("takes"), dir.path().join("cache"));
        Self {
            _dir: dir,
            voice: Arc::new(Counting::default()),
            takes_dir,
            cache_dir,
        }
    }

    fn calls(&self) -> usize {
        self.voice.calls.load(Ordering::SeqCst)
    }

    /// Records a take of line `id` at 16 kHz.
    fn record(&self, id: &str, text: &str) {
        let mut takes = Takes::load(&self.takes_dir).unwrap();
        let pcm = Pcm {
            sample_rate: 16_000,
            channels: 1,
            samples: vec![1; 1_600],
        };
        takes.save(id, text, &pcm).unwrap();
    }

    /// `Lines` over `narration`, with the takes as they are now.
    fn lines(&self, narration: &[NarrationDetail]) -> Lines {
        let mut registry = VoiceRegistry::default();
        registry.register(self.voice.clone());
        let ids: Vec<(String, LineId)> = narration
            .iter()
            .map(|d| (d.backend.clone(), d.line_id.clone()))
            .collect();
        let cast = Backends::from_registry(registry)
            .cast(
                "fake",
                ids.iter().map(|(b, l)| {
                    (
                        b.as_str(),
                        l,
                        SourceSpan {
                            line: 1,
                            column: 1,
                            len: 1,
                        },
                    )
                }),
            )
            .unwrap();
        Lines::new(
            cast,
            VoiceCache::new(&self.cache_dir),
            Takes::load(&self.takes_dir).unwrap(),
        )
    }

    /// Line `id` reading `text`, from its take if one is current.
    fn line(&self, id: &str, text: &str, voice: Option<&str>) -> NarrationDetail {
        let req = SynthRequest {
            text: text.into(),
            locale: "en".into(),
            voice: voice.map(Into::into),
            speed: 1.0,
            instruct: None,
        };
        let take = Takes::load(&self.takes_dir)
            .unwrap()
            .current(id, text)
            .cloned();
        NarrationDetail {
            line_id: LineId::from(id),
            text: text.into(),
            chapter: "intro".into(),
            chapter_index: 0,
            cache_key: teleprompt_voice::cache::key("fake", "1", &req),
            synth_request: req,
            word_timings: None,
            take,
            backend: "fake".into(),
            speaker: None,
            name: None,
        }
    }
}

#[tokio::test]
async fn a_line_is_synthesized_once_then_read_from_the_cache() {
    let f = Fixture::new();
    let line = f.line("a", "Hello there.", None);
    let lines = f.lines(std::slice::from_ref(&line));

    let first = lines.line(&line, false).await.unwrap();
    let again = lines.line(&line, false).await.unwrap();

    assert_eq!(f.calls(), 1);
    assert_eq!(first.wav, again.wav);
    assert_eq!((again.sample_rate, again.duration_ms), (24_000, 100));
}

#[tokio::test]
async fn fresh_asks_the_voice_again() {
    let f = Fixture::new();
    let line = f.line("a", "Hello there.", None);
    let lines = f.lines(std::slice::from_ref(&line));

    lines.line(&line, false).await.unwrap();
    lines.line(&line, true).await.unwrap();

    assert_eq!(f.calls(), 2);
}

#[tokio::test]
async fn a_take_is_played_as_recorded_and_nothing_is_synthesized() {
    let f = Fixture::new();
    f.record("a", "Hello there.");
    let line = f.line("a", "Hello there.", None);
    let lines = f.lines(std::slice::from_ref(&line));

    let audio = lines.line(&line, true).await.unwrap();

    assert_eq!(f.calls(), 0);
    assert_eq!((audio.sample_rate, audio.duration_ms), (16_000, 100));
}

/// The prompter used to read `takes/<id>.wav` itself, and so played a take
/// `dub` refuses.
#[tokio::test]
async fn a_take_changed_since_it_was_recorded_is_refused() {
    let f = Fixture::new();
    f.record("a", "Hello there.");
    let line = f.line("a", "Hello there.", None);
    let lines = f.lines(std::slice::from_ref(&line));
    std::fs::write(f.takes_dir.join("a.wav"), b"not the take").unwrap();

    let e = lines.line(&line, false).await.expect_err("refused");

    assert!(
        e.to_string().contains("has changed since it was recorded"),
        "{e}"
    );
}

#[tokio::test]
async fn all_renders_each_key_once_and_keeps_document_order() {
    let f = Fixture::new();
    f.record("b", "Recorded.");
    let narration = [
        f.line("a", "Same words.", None),
        f.line("b", "Recorded.", None),
        f.line("c", "Same words.", None),
    ];
    let lines = f.lines(&narration);

    let audio = lines.all(&narration, 2, &Silent).await.unwrap();

    assert_eq!(f.calls(), 1, "`a` and `c` share a key");
    let order: Vec<(&str, u32)> = audio
        .iter()
        .map(|a| (a.line_id.as_str(), a.sample_rate))
        .collect();
    assert_eq!(order, [("a", 24_000), ("b", 16_000), ("c", 24_000)]);
}

#[tokio::test]
async fn all_refuses_a_voice_the_server_does_not_offer_before_synthesizing() {
    let f = Fixture::new();
    let narration = [f.line("a", "Hello there.", Some("bob"))];
    let lines = f.lines(&narration);

    let e = lines
        .all(&narration, 1, &Silent)
        .await
        .expect_err("refused");

    assert!(matches!(e, Failure::Validation(_)), "{e}");
    assert!(
        e.to_string().contains("voice `bob` is not available"),
        "{e}"
    );
    assert_eq!(f.calls(), 0);
}
