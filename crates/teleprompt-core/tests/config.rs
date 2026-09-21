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

/// Renamed from `real_front_matter_with_schema_version_and_output_block_\
/// deserializes`: it was named for the general property while using front
/// matter edited down to what already worked. What it actually checks is
/// that `teleprompt:` and the `output:` block reach `Config` — a narrower
/// and still worthwhile claim. The general claim is carried by
/// `the_specs_own_section_3_1_front_matter_deserializes_verbatim` below.
#[test]
fn the_schema_version_and_output_block_reach_the_merged_config() {
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

/// Final review, item 8. Three constructs in the design spec's own canonical
/// example were rejected by the compiler an M1 implementer would be building
/// against:
///
/// ```text
/// voice.synthetic                       -> unknown field `synthetic`
/// scene.default: browser                -> invalid type: string, expected struct PartialScene
/// scene.browser.viewport: [1920, 1080]  -> invalid type: sequence, expected a string
/// ```
///
/// The block below is spec §3.1's front matter pasted **verbatim**,
/// comments and alignment included. It is deliberately not pruned: pruning
/// it is what let the gap survive review the first time.
const SPEC_3_1_FRONT_MATTER: &str = r#"teleprompt: 1
locales:
  source: en
  targets: [nl, fr]
voice:
  source: synthetic                                  # synthetic | cloned | recorded
  synthetic: { backend: kokoro, model: af_heart, speed: 1.0 }
  cloned:    { backend: elevenlabs, profile: jan }   # see voices/jan.toml
  recorded:  { takes_dir: takes }
scene:
  default: browser
  browser:
    base_url: "http://localhost:3000"
    viewport: [1920, 1080]
    device_scale_factor: 2
output:
  resolution: [1920, 1080]
  fps: 30
  transition: { kind: crossfade, duration: auto, max_ms: 600 }
"#;

#[test]
fn the_specs_own_section_3_1_front_matter_deserializes_verbatim() {
    let partial = PartialConfig::from_yaml(SPEC_3_1_FRONT_MATTER)
        .expect("the spec's canonical front matter must deserialize");
    let c = Config::merged(&[partial]);

    assert_eq!(c.locales.source, "en");
    assert_eq!(c.locales.targets, ["nl", "fr"]);
    assert_eq!(c.voice.source, "synthetic");
    assert_eq!(c.transition.kind, "crossfade");
    assert_eq!(c.transition.duration, TransitionDuration::Auto);
    assert_eq!(c.transition.max_ms, 600);

    assert_eq!(c.default_scene.as_deref(), Some("browser"));
    let browser = c.scenes.get("browser").expect("browser scene configured");
    assert_eq!(
        browser.adapter, "playwright",
        "an unconfigured adapter still falls back by scene name"
    );
    assert_eq!(
        browser.settings.get("viewport"),
        Some(&serde_yaml::from_str::<serde_yaml::Value>("[1920, 1080]").unwrap()),
        "structured settings survive instead of being stringified or rejected"
    );
    assert_eq!(
        browser.settings.get("base_url").and_then(|v| v.as_str()),
        Some("http://localhost:3000")
    );
}

/// The per-tier voice blocks are parsed rather than merged — like
/// `output.resolution` and `output.fps`, they are declared so the spec's
/// front matter deserializes and are unused until their backends land in M3.
/// Asserted here so the deferral is recorded rather than assumed.
#[test]
fn the_per_tier_voice_blocks_are_parsed_and_kept() {
    let p = PartialConfig::from_yaml(SPEC_3_1_FRONT_MATTER).unwrap();
    let voice = p.voice.expect("voice block present");
    assert_eq!(
        voice.synthetic.as_ref().unwrap().backend.as_deref(),
        Some("kokoro")
    );
    assert_eq!(
        voice.synthetic.as_ref().unwrap().settings.get("model"),
        Some(&serde_yaml::Value::from("af_heart"))
    );
    assert_eq!(
        voice.cloned.as_ref().unwrap().backend.as_deref(),
        Some("elevenlabs")
    );
    assert_eq!(
        voice.recorded.as_ref().unwrap().settings.get("takes_dir"),
        Some(&serde_yaml::Value::from("takes"))
    );
}

/// Accepting what the spec documents must not mean accepting anything.
/// `scene: { browser: playwright }` is not a shape this grammar has a meaning
/// for, and an untagged "name or settings" enum would have deserialized it to
/// nothing at all.
#[test]
fn a_bare_string_under_a_scene_key_is_still_an_error() {
    let err = PartialConfig::from_yaml("scene:\n  browser: playwright\n").unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("invalid type: string"),
        "a scene key must hold settings, not a name: {msg}"
    );
}

/// And `deny_unknown_fields` still holds at the top level.
#[test]
fn an_unknown_top_level_key_is_still_an_error() {
    let err = PartialConfig::from_yaml("teleprompt: 1\nvioce:\n  source: synthetic\n").unwrap_err();
    assert!(err.to_string().contains("vioce"), "{err}");
}

#[test]
fn backend_settings_survive_the_merge_without_core_understanding_them() {
    let base: PartialConfig = PartialConfig::from_yaml(
        "backends:\n  kokoro:\n    base_url: \"http://localhost:8880\"\n    timeout_ms: 30000\n",
    )
    .unwrap();
    let over: PartialConfig =
        PartialConfig::from_yaml("backends:\n  kokoro:\n    timeout_ms: 5000\n").unwrap();

    let merged = Config::merged(&[base, over]);
    let k = merged.backends.get("kokoro").expect("kokoro settings kept");

    // Per-key merge, not whole-table replacement: the later layer overrides
    // `timeout_ms` and leaves `base_url` alone. Whole-table replacement would
    // mean a script overriding one setting silently discards the project's
    // other ones.
    assert_eq!(k.get("timeout_ms").unwrap().as_u64(), Some(5000));
    assert_eq!(
        k.get("base_url").unwrap().as_str(),
        Some("http://localhost:8880")
    );
}

#[test]
fn an_unknown_backend_table_is_not_an_error() {
    // Core does not validate backend ids. A config naming a backend this
    // build does not ship must parse; `voice.backend` selection is where an
    // unknown id is reported, with the available list.
    let c = PartialConfig::from_yaml("backends:\n  elevenlabs:\n    profile: jan\n").unwrap();
    let merged = Config::merged(&[c]);
    assert!(merged.backends.contains_key("elevenlabs"));
}

/// `output.resolution` and `output.fps` were parsed and dropped for as long
/// as nothing rendered. They resolve like every other setting now: the
/// later layer wins, and a layer that says nothing changes nothing.
#[test]
fn the_output_shape_resolves_through_the_layers() {
    let default = Config::merged(&[]);
    assert_eq!(default.output.resolution, (1920, 1080));
    assert_eq!(default.output.fps, 30);

    let project = PartialConfig::from_yaml("output:\n  resolution: [1280, 720]\n  fps: 24\n")
        .expect("valid YAML");
    let script = PartialConfig::from_yaml("output:\n  fps: 60\n").expect("valid YAML");

    let merged = Config::merged(&[project, script]);
    assert_eq!(
        merged.output.resolution,
        (1280, 720),
        "the script said nothing about size, so the project's stands"
    );
    assert_eq!(merged.output.fps, 60, "and its frame rate overrides");
}

/// Half a resolution is not a size, and silently ignoring it would render
/// at 1080p while the author believed otherwise — visible only after the
/// wait.
#[test]
fn a_resolution_that_is_not_a_pair_is_rejected() {
    let bad = PartialConfig::from_yaml("output:\n  resolution: [1920]\n");
    let message = bad
        .expect_err("a one-element resolution is not one")
        .to_string();
    assert!(
        message.contains("resolution"),
        "the error names the field: {message}"
    );
}

/// Zero deserializes perfectly well and divides just as badly: a zero frame
/// rate is a render that cannot be scheduled and a zero-width frame is one
/// no encoder will accept.
#[test]
fn a_frame_that_cannot_exist_is_reported_by_the_merged_config() {
    let zero_fps = Config::merged(&[PartialConfig::from_yaml("output:\n  fps: 0\n").unwrap()]);
    assert!(
        zero_fps.problems().iter().any(|p| p.contains("output.fps")),
        "{:?}",
        zero_fps.problems()
    );

    let zero_size =
        Config::merged(&[PartialConfig::from_yaml("output:\n  resolution: [0, 1080]\n").unwrap()]);
    assert!(
        zero_size
            .problems()
            .iter()
            .any(|p| p.contains("output.resolution")),
        "{:?}",
        zero_size.problems()
    );
}

/// A pronunciation map is per-project and per-script: `teleprompt.toml`
/// carries the house words, front matter adds the ones a single script
/// needs, and the two merge rather than replacing one another.
#[test]
fn pronunciations_accumulate_across_the_layers() {
    let project =
        PartialConfig::from_yaml("voice:\n  pronounce:\n    MWAA: em-double-you-ay-ay\n").unwrap();
    let script =
        PartialConfig::from_yaml("voice:\n  pronounce:\n    flowrs: flow-ers\n    MWAA: moo-ah\n")
            .unwrap();

    let merged = Config::merged(&[project, script]);
    assert_eq!(merged.voice.pronounce.get("flowrs").unwrap(), "flow-ers");
    assert_eq!(
        merged.voice.pronounce.get("MWAA").unwrap(),
        "moo-ah",
        "the later layer wins on the word it names"
    );
    assert_eq!(Config::merged(&[]).voice.pronounce.len(), 0);
}
