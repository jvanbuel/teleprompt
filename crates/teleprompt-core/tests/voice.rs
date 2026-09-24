use teleprompt_core::{VoiceSource, VoiceTier};

/// A downgrade goes down the ladder; the same tier, or a higher one, is not
/// one and cannot be built.
#[test]
fn a_downgrade_must_be_to_a_lower_tier() {
    use VoiceSource::{Cloned, Recorded, Synthetic};
    for (from, to) in [
        (Recorded, Cloned),
        (Recorded, Synthetic),
        (Cloned, Synthetic),
    ] {
        let tier = VoiceTier::downgraded(from, to, "why".into()).expect("a real downgrade");
        assert_eq!(
            (tier.requested(), tier.actual(), tier.reason()),
            (from, to, Some("why"))
        );
    }
    for (from, to) in [
        (Synthetic, Synthetic),
        (Synthetic, Recorded),
        (Cloned, Recorded),
    ] {
        assert!(
            VoiceTier::downgraded(from, to, "why".into()).is_err(),
            "{from} → {to}"
        );
    }
}

#[test]
fn a_delivered_tier_has_no_reason() {
    let tier = VoiceTier::delivered(VoiceSource::Cloned);
    assert_eq!(tier.actual(), VoiceSource::Cloned);
    assert_eq!(tier.downgrade(), None);
}
