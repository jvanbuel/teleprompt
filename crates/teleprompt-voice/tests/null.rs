use teleprompt_voice::{NullVoice, SynthRequest, VoiceBackend};

fn req(text: &str) -> SynthRequest {
    SynthRequest {
        text: text.into(),
        locale: "en".into(),
        voice: None,
        speed: 1.0,
    }
}

#[test]
fn duration_scales_with_word_count() {
    let v = NullVoice::default();
    let short = v.synthesize(&req("one two three")).unwrap().duration_ms;
    let long = v
        .synthesize(&req("one two three four five six"))
        .unwrap()
        .duration_ms;
    assert!(long > short);
}

#[test]
fn six_words_at_150_wpm_is_2400ms_plus_no_punctuation() {
    let v = NullVoice { wpm: 150.0 };
    // 6 words / 150 wpm * 60_000 = 2400 ms
    assert_eq!(
        v.synthesize(&req("one two three four five six"))
            .unwrap()
            .duration_ms,
        2400
    );
}

#[test]
fn punctuation_adds_pauses() {
    let v = NullVoice { wpm: 150.0 };
    let plain = v.synthesize(&req("one two")).unwrap().duration_ms;
    let stopped = v.synthesize(&req("one two.")).unwrap().duration_ms;
    assert_eq!(stopped - plain, 350);

    let comma = v.synthesize(&req("one, two")).unwrap().duration_ms;
    assert_eq!(comma - plain, 150);
}

#[test]
fn speed_divides_the_duration() {
    let v = NullVoice { wpm: 150.0 };
    let mut r = req("one two three four five six");
    r.speed = 2.0;
    assert_eq!(v.synthesize(&r).unwrap().duration_ms, 1200);
}

#[test]
fn synthesis_is_deterministic() {
    let v = NullVoice::default();
    let a = v.synthesize(&req("Welcome to Acme.")).unwrap();
    let b = v.synthesize(&req("Welcome to Acme.")).unwrap();
    assert_eq!(a.duration_ms, b.duration_ms);
    assert_eq!(a.audio_hash, b.audio_hash);
}

#[test]
fn cache_key_varies_with_every_input_that_changes_the_output() {
    let v = NullVoice::default();
    let base = req("hello");
    let mut other_locale = base.clone();
    other_locale.locale = "nl".into();
    let mut other_speed = base.clone();
    other_speed.speed = 1.5;

    assert_ne!(v.cache_key(&base), v.cache_key(&other_locale));
    assert_ne!(v.cache_key(&base), v.cache_key(&other_speed));
    assert_eq!(v.cache_key(&base), v.cache_key(&req("hello")));
}

#[test]
fn empty_text_is_zero_duration() {
    let v = NullVoice::default();
    assert_eq!(v.synthesize(&req("   ")).unwrap().duration_ms, 0);
}

#[test]
fn null_backend_declares_honest_capabilities() {
    let c = NullVoice::default().capabilities();
    assert!(!c.cloning);
    assert!(!c.word_timings);
    assert!(c.speed_control);
}
