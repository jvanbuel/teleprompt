use teleprompt_voice::{resolve_source, VoiceSource};

#[test]
fn available_tier_is_used_unchanged() {
    let r = resolve_source(VoiceSource::Recorded, &|_| Ok(())).unwrap();
    assert_eq!(r.actual, VoiceSource::Recorded);
    assert!(r.downgrade_reason.is_none());
}

#[test]
fn unavailable_tier_drops_one_step_and_records_why() {
    let r = resolve_source(VoiceSource::Recorded, &|t| match t {
        VoiceSource::Recorded => Err("take stale".into()),
        _ => Ok(()),
    })
    .unwrap();
    assert_eq!(r.requested, VoiceSource::Recorded);
    assert_eq!(r.actual, VoiceSource::Cloned);
    assert_eq!(r.downgrade_reason.as_deref(), Some("take stale"));
}

#[test]
fn the_ladder_descends_more_than_one_rung_when_needed() {
    let r = resolve_source(VoiceSource::Recorded, &|t| match t {
        VoiceSource::Synthetic => Ok(()),
        _ => Err("unavailable".into()),
    })
    .unwrap();
    assert_eq!(r.actual, VoiceSource::Synthetic);
}

#[test]
fn exhausting_the_ladder_is_an_error() {
    let e = resolve_source(VoiceSource::Recorded, &|_| Err("nope".into())).unwrap_err();
    assert!(e.contains("no voice source available"));
}

#[test]
fn a_lower_request_never_climbs_back_up() {
    let r = resolve_source(VoiceSource::Synthetic, &|_| Ok(())).unwrap();
    assert_eq!(r.actual, VoiceSource::Synthetic);
}
