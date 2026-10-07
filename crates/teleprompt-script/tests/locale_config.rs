//! `[locale.<code>]`: settings for one locale, over the layer they are in.

use teleprompt_script::config::PartialConfig;
use teleprompt_script::parse::parse_script;
use teleprompt_script::program::{resolve, Element, Program};

const PROJECT: &str = r#"
[voice]
backend = "null"
voice = "af_heart"
speed = 1.0

[locale.nl.voice]
voice = "nl_voice"
"#;

fn program(front: &str, locale: &str) -> Program {
    let src = format!("---\nteleprompt: 1\n{front}---\n\n# A\n\nOne. {{#one}}\n");
    let s = parse_script(&src).unwrap();
    let project = PartialConfig::from_toml(PROJECT).unwrap();
    resolve(&s, "a.md", locale, &project, &PartialConfig::default()).unwrap()
}

fn voice(p: &Program) -> (Option<String>, f64) {
    match &p.elements[0] {
        Element::Narration { config, .. } => (config.voice.voice.clone(), config.voice.speed),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_locale_section_applies_only_to_its_locale() {
    assert_eq!(voice(&program("", "en")), (Some("af_heart".into()), 1.0));
    assert_eq!(voice(&program("", "nl")), (Some("nl_voice".into()), 1.0));
    assert_eq!(
        program("", "nl").config.voice.voice.as_deref(),
        Some("nl_voice")
    );
}

/// Front matter's locale section is over the front matter, which is over
/// the project's: a script can slow its Dutch down without naming a voice.
#[test]
fn front_matter_can_have_locale_sections_too() {
    let front = "voice:\n  speed: 1.2\nlocale:\n  nl:\n    voice:\n      speed: 0.9\n";
    assert_eq!(voice(&program(front, "en")), (Some("af_heart".into()), 1.2));
    assert_eq!(voice(&program(front, "nl")), (Some("nl_voice".into()), 0.9));
}

/// `[translate]` picks the translator, local by default, and can differ
/// per target language.
#[test]
fn translate_settings_default_to_a_local_model_and_layer_like_the_rest() {
    let defaults = teleprompt_script::config::Config::default();
    assert_eq!(defaults.translate.provider, "ollama");
    assert_eq!(defaults.translate.model, None);
    assert_eq!(defaults.translate.timeout_ms, 600_000);

    let project = PartialConfig::from_toml(
        "[translate]\nprovider = \"openai\"\nmodel = \"local-model\"\ntimeout_ms = 60000\n\n\
         [locale.ja.translate]\nmodel = \"bigger-model\"\ntimeout_ms = 900000\n",
    )
    .unwrap();
    let merged =
        |locale: &str| teleprompt_script::config::Config::merged(&project.in_locale(locale));
    assert_eq!(merged("nl").translate.provider, "openai");
    assert_eq!(merged("nl").translate.model.as_deref(), Some("local-model"));
    assert_eq!(
        merged("ja").translate.model.as_deref(),
        Some("bigger-model")
    );
    assert_eq!(merged("nl").translate.timeout_ms, 60_000);
    assert_eq!(merged("ja").translate.timeout_ms, 900_000);
}
