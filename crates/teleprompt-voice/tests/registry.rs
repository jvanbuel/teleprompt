use std::sync::Arc;
use teleprompt_voice::VoiceRegistry;

mod stub;
use stub::StubVoice;

#[test]
fn available_is_sorted_so_listings_are_deterministic() {
    let mut r = VoiceRegistry::default();
    r.register(Arc::new(StubVoice::new("zulu")));
    r.register(Arc::new(StubVoice::new("alpha")));
    r.register(Arc::new(StubVoice::new("mike")));
    assert_eq!(r.available(), vec!["alpha", "mike", "zulu"]);
}

#[test]
fn get_returns_the_registered_backend_and_none_otherwise() {
    let mut r = VoiceRegistry::default();
    r.register(Arc::new(StubVoice::new("alpha")));
    assert_eq!(
        r.get("alpha").map(|b| b.id().to_string()),
        Some("alpha".to_string())
    );
    assert!(r.get("missing").is_none());
}

#[test]
fn registering_the_same_id_twice_replaces_it() {
    let mut r = VoiceRegistry::default();
    r.register(Arc::new(StubVoice::new("alpha")));
    r.register(Arc::new(StubVoice::new("alpha")));
    assert_eq!(r.available(), vec!["alpha"], "one entry, not two");
}
