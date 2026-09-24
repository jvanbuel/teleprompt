use teleprompt_core::attrs::{parse_attrs, BLOCK_KEYS, SEGMENT_KEYS};
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
    let (a, _) = parse_attrs("lead_in=250ms tail=1s", SEGMENT_KEYS, SPAN);
    assert_eq!(a.get_ms("lead_in"), Some(Ok(250)));
    assert_eq!(a.get_ms("tail"), Some(Ok(1000)));
}

#[test]
fn malformed_duration_reports_an_error_value() {
    let (a, _) = parse_attrs("lead_in=soon", SEGMENT_KEYS, SPAN);
    assert!(a.get_ms("lead_in").unwrap().is_err());
}

#[test]
fn float_values_parse() {
    let (a, _) = parse_attrs("max_stretch=2.5", BLOCK_KEYS, SPAN);
    assert_eq!(a.get_f64("max_stretch"), Some(Ok(2.5)));
}

#[test]
fn bare_token_without_equals_is_an_error() {
    let (_, d) = parse_attrs("policy", BLOCK_KEYS, SPAN);
    assert!(d[0].message.contains("expected `key=value`"));
}

/// A duration attribute that does not parse, or exceeds a day, is reported
/// at the line rather than silently ignored (#22): `from_attrs` drops such
/// values on the promise that this has already said so.
#[test]
fn a_bad_duration_value_is_a_diagnostic() {
    for raw in [
        "lead_in=soon",
        "tail=18446744073709551615ms",
        "lead_in=90000s",
    ] {
        let (_, d) = parse_attrs(raw, SEGMENT_KEYS, SPAN);
        assert_eq!(d.len(), 1, "{raw}: {d:?}");
        assert_eq!(d[0].span, Some(SPAN), "{raw}");
    }
}
