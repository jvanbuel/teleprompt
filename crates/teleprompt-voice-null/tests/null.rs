use teleprompt_voice::estimator::DurationEstimator;
use teleprompt_voice::{Pcm, SynthRequest, VoiceBackend};
use teleprompt_voice_null::{NullVoice, WpmEstimator, NULL_SAMPLE_RATE};

fn req(text: &str) -> SynthRequest {
    SynthRequest {
        text: text.into(),
        locale: "en".into(),
        voice: None,
        speed: 1.0,
    }
}

#[tokio::test]
async fn duration_scales_with_word_count() {
    let v = NullVoice::default();
    let short = v
        .synthesize(&req("one two three"))
        .await
        .unwrap()
        .pcm
        .duration_ms();
    let long = v
        .synthesize(&req("one two three four five six"))
        .await
        .unwrap()
        .pcm
        .duration_ms();
    assert!(long > short);
}

#[tokio::test]
async fn six_words_at_150_wpm_is_2400ms_plus_no_punctuation() {
    let v = NullVoice { wpm: 150.0 };
    // 6 words / 150 wpm * 60_000 = 2400 ms
    assert_eq!(
        v.synthesize(&req("one two three four five six"))
            .await
            .unwrap()
            .pcm
            .duration_ms(),
        2400
    );
}

#[tokio::test]
async fn punctuation_adds_pauses() {
    let v = NullVoice { wpm: 150.0 };
    let plain = v
        .synthesize(&req("one two"))
        .await
        .unwrap()
        .pcm
        .duration_ms();
    let stopped = v
        .synthesize(&req("one two."))
        .await
        .unwrap()
        .pcm
        .duration_ms();
    assert_eq!(stopped - plain, 350);

    let comma = v
        .synthesize(&req("one, two"))
        .await
        .unwrap()
        .pcm
        .duration_ms();
    assert_eq!(comma - plain, 150);
}

#[tokio::test]
async fn speed_divides_the_duration() {
    let v = NullVoice { wpm: 150.0 };
    let mut r = req("one two three four five six");
    r.speed = 2.0;
    assert_eq!(v.synthesize(&r).await.unwrap().pcm.duration_ms(), 1200);
}

#[tokio::test]
async fn synthesis_is_deterministic() {
    let v = NullVoice::default();
    let a = v.synthesize(&req("Welcome to Acme.")).await.unwrap();
    let b = v.synthesize(&req("Welcome to Acme.")).await.unwrap();
    assert_eq!(a.pcm, b.pcm);
}

#[tokio::test]
async fn null_backend_declares_honest_capabilities() {
    let c = NullVoice::default().capabilities();
    assert!(!c.cloning);
    assert!(!c.word_timings);
    assert!(c.speed_control);
}

#[tokio::test]
async fn punctuation_only_text_is_silent() {
    let v = NullVoice::default();
    assert_eq!(
        v.synthesize(&req("...")).await.unwrap().pcm.duration_ms(),
        0
    );
    assert_eq!(
        v.synthesize(&req("-- !! --"))
            .await
            .unwrap()
            .pcm
            .duration_ms(),
        0
    );
}

#[tokio::test]
async fn wpm_affects_duration() {
    let slow = NullVoice { wpm: 150.0 };
    let fast = NullVoice { wpm: 300.0 };
    let same_as_slow = NullVoice { wpm: 150.0 };
    let r = req("hello world");

    assert_ne!(
        slow.synthesize(&r).await.unwrap().pcm.duration_ms(),
        fast.synthesize(&r).await.unwrap().pcm.duration_ms()
    );
    assert_eq!(
        slow.synthesize(&r).await.unwrap().pcm.duration_ms(),
        same_as_slow.synthesize(&r).await.unwrap().pcm.duration_ms()
    );
}

#[tokio::test]
async fn null_synthesizes_silence_matching_the_estimate() {
    let v = NullVoice::default();
    let r = req("Every video in this repository is built from a script you can read.");

    let out = v.synthesize(&r).await.unwrap();

    assert_eq!(out.pcm.sample_rate, NULL_SAMPLE_RATE);
    assert_eq!(out.pcm.channels, 1);
    assert!(out.pcm.samples.iter().all(|s| *s == 0), "null is silence");
    assert_eq!(
        out.pcm.duration_ms(),
        WpmEstimator::default().estimate_ms(&r),
        "the audio's length is the estimate — one number, not two"
    );
    assert!(out.word_timings.is_none());
}

#[tokio::test]
async fn null_rejects_a_non_positive_speed() {
    let v = NullVoice::default();
    let mut r = req("hello");
    r.speed = 0.0;
    assert!(v.synthesize(&r).await.is_err());
}

#[tokio::test]
async fn empty_text_synthesizes_nothing() {
    let v = NullVoice::default();
    let out = v.synthesize(&req("   ")).await.unwrap();
    assert_eq!(out.pcm.samples.len(), 0);
    assert_eq!(out.pcm.duration_ms(), 0);
}

#[test]
fn pcm_duration_is_computed_per_frame_not_per_sample() {
    let stereo = Pcm {
        sample_rate: 48_000,
        channels: 2,
        samples: vec![0; 96_000], // 48_000 frames = 1000 ms
    };
    assert_eq!(stereo.duration_ms(), 1000);
}

/// The invariant the whole workspace leans on without saying so: `null`'s
/// audio is *exactly* what the default estimator predicts, so a warm cache
/// and a cold one produce the same numbers and several tests elsewhere read
/// as healthy. It used to be maintained by two unrelated `150.0` literals in
/// two files.
///
/// Asserted against rendered audio rather than by comparing the constants,
/// which would be a tautology: the claim is about the length of the samples
/// that come out, and the rate is only one of the two ways it could break —
/// the frame arithmetic in `synthesize` is the other.
#[tokio::test]
async fn null_renders_exactly_the_default_estimate() {
    let sentence = "Every video in this repository is built from a script you can read, \
                    and the timeline is committed beside it; nothing here is a guess.";

    for speed in [0.5, 1.0, 2.0] {
        let mut r = req(sentence);
        r.speed = speed;
        let rendered = NullVoice::default()
            .synthesize(&r)
            .await
            .unwrap()
            .pcm
            .duration_ms();
        let estimated = WpmEstimator::default().estimate_ms(&r);
        assert!(estimated > 0, "the fixture must be non-trivial");
        assert_eq!(
            rendered, estimated,
            "at speed {speed}, null's audio must be exactly the default estimate"
        );
    }
}
