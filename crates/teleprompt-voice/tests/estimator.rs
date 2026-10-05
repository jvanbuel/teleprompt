use teleprompt_voice::SynthRequest;
use teleprompt_voice::WpmEstimator;

fn req(text: &str, speed: f64) -> SynthRequest {
    SynthRequest {
        text: text.to_string(),
        locale: "en".to_string(),
        voice: None,
        speed,
        instruct: None,
    }
}

#[test]
fn six_words_at_150_wpm_is_2400ms() {
    let e = WpmEstimator::default();
    assert_eq!(
        e.estimate_ms(&req("one two three four five six", 1.0)),
        2400
    );
}

#[test]
fn punctuation_adds_pauses() {
    let e = WpmEstimator::default();
    let plain = e.estimate_ms(&req("one two three four five six", 1.0));
    let punctuated = e.estimate_ms(&req("one two three, four five six.", 1.0));
    assert_eq!(
        punctuated,
        plain + 150 + 350,
        "comma 150ms, sentence end 350ms"
    );
}

#[test]
fn speed_divides_the_duration() {
    let e = WpmEstimator::default();
    let normal = e.estimate_ms(&req("one two three four five six", 1.0));
    assert_eq!(
        e.estimate_ms(&req("one two three four five six", 2.0)),
        normal / 2
    );
}

#[test]
fn estimation_is_deterministic() {
    let e = WpmEstimator::default();
    let r = req("Every video here is built from a script.", 1.0);
    assert_eq!(e.estimate_ms(&r), e.estimate_ms(&r));
}
