use teleprompt_core::attrs::{parse_attrs, BlockAttrs, LineAttrs, BLOCK_KEYS, SEGMENT_KEYS};
use teleprompt_core::policy::{Align, PolicyKind};
use teleprompt_core::DurationMs;
use teleprompt_core::SourceSpan;

const SPAN: SourceSpan = SourceSpan {
    line: 1,
    column: 1,
    len: 0,
};

#[test]
fn parses_key_value_pairs() {
    let (a, d) = parse_attrs("policy=concurrent align=start", BLOCK_KEYS, SPAN);
    assert!(d.is_empty());
    assert_eq!(a.get("policy"), Some("concurrent"));
    assert_eq!(a.get("align"), Some("start"));
}

#[test]
fn ignores_the_leading_id_token() {
    let (a, d) = parse_attrs("#welcome voice.source=recorded", SEGMENT_KEYS, SPAN);
    assert!(d.is_empty());
    assert_eq!(a.get("voice.source"), Some("recorded"));
}

#[test]
fn accepts_quoted_values_with_spaces() {
    let (a, d) = parse_attrs(r#"include="scripts/my demo.spec.ts""#, BLOCK_KEYS, SPAN);
    assert!(d.is_empty());
    assert_eq!(a.get("include"), Some("scripts/my demo.spec.ts"));
}

#[test]
fn unknown_key_is_an_error_with_a_suggestion() {
    let (_, d) = parse_attrs("polcy=hold", BLOCK_KEYS, SPAN);
    assert_eq!(d.len(), 1);
    assert!(d[0].is_error());
    assert!(d[0].message.contains("unknown attribute key `polcy`"));
    assert_eq!(d[0].help.as_deref(), Some("did you mean `policy`?"));
}

#[test]
fn unknown_key_with_no_near_match_has_no_suggestion() {
    let (_, d) = parse_attrs("zzzzzz=1", BLOCK_KEYS, SPAN);
    assert_eq!(d.len(), 1);
    assert!(d[0].help.is_none());
}

#[test]
fn a_key_valid_elsewhere_is_still_rejected_here() {
    let (_, d) = parse_attrs("policy=hold", SEGMENT_KEYS, SPAN);
    assert!(d[0].message.contains("unknown attribute key `policy`"));
}

#[test]
fn duration_values_parse_from_ms_and_s() {
    let (a, d) = LineAttrs::parse("lead_in=250ms tail=1s", SPAN);
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(a.lead_in, Some(DurationMs::millis(250)));
    assert_eq!(a.tail, Some(DurationMs::millis(1000)));
}

#[test]
fn a_malformed_duration_is_absent_from_the_parsed_attributes() {
    let (a, _) = LineAttrs::parse("lead_in=soon", SPAN);
    assert_eq!(a.lead_in, None);
}

#[test]
fn float_values_parse() {
    let (a, d) = BlockAttrs::parse("max_stretch=2.5", SPAN);
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(a.max_stretch, Some(2.5));
}

#[test]
fn bare_token_without_equals_is_an_error() {
    let (_, d) = parse_attrs("policy", BLOCK_KEYS, SPAN);
    assert!(d[0].message.contains("expected `key=value`"));
}

/// A duration attribute that does not parse, or exceeds a day, is reported
/// at the line rather than silently ignored (#22).
#[test]
fn a_bad_duration_value_is_a_diagnostic() {
    for raw in [
        "lead_in=soon",
        "tail=18446744073709551615ms",
        "lead_in=90000s",
    ] {
        let (_, d) = LineAttrs::parse(raw, SPAN);
        assert_eq!(d.len(), 1, "{raw}: {d:?}");
        assert_eq!(d[0].span, Some(SPAN), "{raw}");
    }
}

/// A number that does not parse is reported at its line like a duration,
/// not dropped: `voice.speed=fast` used to leave the speed at its default
/// without a word.
#[test]
fn a_bad_number_value_is_a_diagnostic() {
    let (_, d) = LineAttrs::parse("voice.speed=fast", SPAN);
    assert_eq!(d.len(), 1, "{d:?}");
    assert!(d[0].message.contains("not a number"), "{}", d[0].message);
    for raw in ["max_stretch=lots", "min_stretch=", "max_speedup=2x"] {
        let (_, d) = BlockAttrs::parse(raw, SPAN);
        assert_eq!(d.len(), 1, "{raw}: {d:?}");
        assert!(
            d[0].message.contains("not a number"),
            "{raw}: {}",
            d[0].message
        );
    }
}

/// Policy and align are read at the block like any other value, so a bad
/// one is reported where it was written; compile used to report it with no
/// position at all.
#[test]
fn policy_and_align_are_parsed_at_the_block() {
    let (a, d) = BlockAttrs::parse("policy=concurrent align=end", SPAN);
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(a.policy, Some(PolicyKind::Concurrent));
    assert_eq!(a.align, Some(Align::End));

    for (raw, says) in [
        ("policy=sideways", "unknown policy `sideways`"),
        ("policy=stretch", "renamed to `stretch-action`"),
        ("align=middle", "unknown align `middle`"),
    ] {
        let (_, d) = BlockAttrs::parse(raw, SPAN);
        assert_eq!(d.len(), 1, "{raw}: {d:?}");
        assert!(d[0].message.contains(says), "{raw}: {}", d[0].message);
        assert_eq!(d[0].span, Some(SPAN), "{raw}");
    }
}

/// Every policy name parses, and the retired spellings are errors naming
/// their replacement rather than aliases (issue #1: `stretch` and `trim`
/// named an operation without its object, so both read as if the speech
/// were adjusted).
#[test]
fn policy_names_parse_and_the_old_ones_name_their_replacement() {
    for (name, kind) in [
        ("hold", PolicyKind::Hold),
        ("concurrent", PolicyKind::Concurrent),
        ("stretch-action", PolicyKind::StretchAction),
        ("trim-action", PolicyKind::TrimAction),
    ] {
        assert_eq!(PolicyKind::parse(name), Ok(kind));
        assert_eq!(kind.label(), name);
    }
    for (old, new) in [("stretch", "stretch-action"), ("trim", "trim-action")] {
        let (message, help) = PolicyKind::parse(old).unwrap_err();
        assert!(message.contains(new), "{message}");
        assert!(help.contains(&format!("policy={new}")), "{help}");
    }
    assert!(PolicyKind::parse("nonsense").is_err());
}
