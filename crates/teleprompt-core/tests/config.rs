use teleprompt_core::attrs::{parse_attrs, SEGMENT_KEYS};
use teleprompt_core::config::{Config, PartialConfig, TransitionDuration};
use teleprompt_core::SourceSpan;

#[test]
fn defaults_match_the_spec() {
    let c = Config::default();
    assert_eq!(c.timing.lead_in_ms, 150);
    assert_eq!(c.timing.tail_ms, 150);
    assert_eq!(c.timing.max_stretch, 3.0);
    assert_eq!(c.timing.min_stretch, 0.33);
    assert_eq!(c.timing.max_speedup, 2.0);
    assert_eq!(c.transition.max_ms, 600);
    assert_eq!(c.transition.min_ms, 0);
    assert_eq!(c.transition.duration, TransitionDuration::Auto);
    assert_eq!(c.locales.source, "en");
}

#[test]
fn later_layers_override_earlier_ones_field_by_field() {
    let project = PartialConfig::from_toml("[timing]\nlead_in_ms = 300\ntail_ms = 400\n").unwrap();
    let script = PartialConfig::from_yaml("timing:\n  tail_ms: 500\n").unwrap();
    let c = Config::merged(&[project, script]);
    assert_eq!(c.timing.lead_in_ms, 300, "untouched field survives");
    assert_eq!(c.timing.tail_ms, 500, "later layer wins");
}

#[test]
fn absent_layers_change_nothing() {
    let c = Config::merged(&[PartialConfig::default(), PartialConfig::default()]);
    assert_eq!(c, Config::default());
}

#[test]
fn attributes_become_a_config_layer() {
    let span = SourceSpan {
        line: 1,
        column: 1,
        len: 0,
    };
    let (a, _) = parse_attrs("lead_in=400ms voice.source=cloned", SEGMENT_KEYS, span);
    let c = Config::merged(&[PartialConfig::from_attrs(&a)]);
    assert_eq!(c.timing.lead_in_ms, 400);
    assert_eq!(c.voice.source, "cloned");
}

#[test]
fn scene_config_carries_an_adapter_and_free_form_settings() {
    let yaml = "scene:\n  browser:\n    adapter: playwright\n    base_url: http://localhost:3000\n";
    let c = Config::merged(&[PartialConfig::from_yaml(yaml).unwrap()]);
    let s = c.scenes.get("browser").unwrap();
    assert_eq!(s.adapter, "playwright");
    assert_eq!(s.settings.get("base_url").unwrap(), "http://localhost:3000");
}

#[test]
fn scene_adapter_defaults_by_scene_name() {
    let c = Config::merged(&[PartialConfig::from_yaml("scene:\n  terminal: {}\n").unwrap()]);
    assert_eq!(c.scenes.get("terminal").unwrap().adapter, "vhs");
}

#[test]
fn malformed_yaml_is_reported_not_panicked() {
    assert!(PartialConfig::from_yaml("timing:\n  lead_in_ms: [nope\n").is_err());
}

#[test]
fn transition_duration_accepts_auto_or_a_number() {
    let auto = PartialConfig::from_yaml("output:\n  transition:\n    duration: auto\n").unwrap();
    assert_eq!(
        Config::merged(&[auto]).transition.duration,
        TransitionDuration::Auto
    );

    let fixed = PartialConfig::from_yaml("output:\n  transition:\n    duration: 250ms\n").unwrap();
    assert_eq!(
        Config::merged(&[fixed]).transition.duration,
        TransitionDuration::Fixed(250)
    );
}

#[test]
fn real_front_matter_with_schema_version_and_output_block_deserializes() {
    let yaml = "teleprompt: 1\nlocales:\n  source: en\noutput:\n  resolution: [1920, 1080]\n  fps: 30\n  transition: { duration: auto, max_ms: 600 }\nscene:\n  mock: { adapter: mock }\n";
    let c = Config::merged(&[PartialConfig::from_yaml(yaml).unwrap()]);
    assert_eq!(c.transition.max_ms, 600);
    assert_eq!(c.locales.source, "en");
}

#[test]
fn malformed_transition_duration_is_a_reported_error_not_a_silent_auto() {
    let err =
        PartialConfig::from_yaml("output:\n  transition:\n    duration: 25oms\n").unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("25oms"),
        "error should name the offending value, got: {message}"
    );
}

#[test]
fn transition_duration_auto_is_case_insensitive() {
    let upper = PartialConfig::from_yaml("output:\n  transition:\n    duration: AUTO\n").unwrap();
    assert_eq!(
        Config::merged(&[upper]).transition.duration,
        TransitionDuration::Auto
    );

    let mixed = PartialConfig::from_yaml("output:\n  transition:\n    duration: Auto\n").unwrap();
    assert_eq!(
        Config::merged(&[mixed]).transition.duration,
        TransitionDuration::Auto
    );
}
