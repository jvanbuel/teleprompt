//! Speakers: a cast of named voices, and a bold label, `**Guest:**`,
//! opening a line saying who speaks it. A speaker's voice applies over the
//! chapter's and under the line's own attributes; a line without one is
//! the narrator's. And a chapter's settings on its heading.

use teleprompt_core::attrs::{BlockAttrs, LineAttrs};
use teleprompt_core::config::PartialConfig;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::{resolve, Element, Program};
use teleprompt_core::SourceSpan;

const PROJECT: &str = r#"
[voice]
backend = "kokoro"
voice = "af_heart"

[voices.guest]
backend = "gemini"
voice = "Puck"
instruct = "dry, a little deadpan"

[voices.me]
voice = "am_adam"
"#;

fn texts(p: &Program) -> Vec<String> {
    p.elements
        .iter()
        .filter_map(|e| match e {
            Element::Narration { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn resolved(src: &str) -> Result<Program, Vec<String>> {
    let parsed =
        parse_script(src).map_err(|d| d.0.iter().map(|d| d.message.clone()).collect::<Vec<_>>())?;
    let project = PartialConfig::from_toml(PROJECT).unwrap();
    resolve(
        &parsed,
        "talk.md",
        "en",
        &project,
        &PartialConfig::default(),
    )
    .map_err(|d| d.0.iter().map(|d| d.message.clone()).collect())
}

/// A line's speaker, backend, voice and instruction.
type Voiced = (Option<String>, String, Option<String>, Option<String>);

fn voices(p: &Program) -> Vec<Voiced> {
    p.elements
        .iter()
        .filter_map(|e| match e {
            Element::Narration {
                speaker, config, ..
            } => Some((
                speaker.clone(),
                config.voice.backend.clone(),
                config.voice.voice.clone(),
                config.voice.instruct.clone(),
            )),
            _ => None,
        })
        .collect()
}

fn s(v: &str) -> Option<String> {
    Some(v.to_string())
}

#[test]
fn a_line_names_its_speaker_and_gets_their_voice() {
    let p = resolved(
        "# Talk\n\nWelcome back. {#intro}\n\n**Guest:** Only on Fridays. {#friday}\n\n\
         **Me**: Why?\n\n**guest:** Nobody's watching. {#because voice.instruct=conspiratorial}\n",
    )
    .unwrap();
    assert_eq!(
        voices(&p),
        vec![
            (None, "kokoro".into(), s("af_heart"), None),
            (
                s("guest"),
                "gemini".into(),
                s("Puck"),
                s("dry, a little deadpan")
            ),
            // A speaker sets only what it names; the rest is the narrator's.
            (s("me"), "kokoro".into(), s("am_adam"), None),
            // The line's own attributes win over its speaker's.
            (s("guest"), "gemini".into(), s("Puck"), s("conspiratorial")),
        ]
    );
    // The label is who, not what: it is not said.
    assert_eq!(
        texts(&p),
        [
            "Welcome back.",
            "Only on Fridays.",
            "Why?",
            "Nobody's watching."
        ]
    );
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
}

#[test]
fn a_chapter_or_script_names_the_speaker_of_its_lines() {
    let p = resolved(
        "---\nspeaker: me\n---\n\n# Mine\n\nHello. {#a}\n\n# Theirs {speaker=guest}\n\n\
         Hi. {#b}\n\n**Me:** And me. {#c}\n\n# Block\n\n```yaml teleprompt\nspeaker: guest\n```\n\nYo. {#d}\n",
    )
    .unwrap();
    let speakers: Vec<_> = voices(&p).into_iter().map(|v| v.0).collect();
    assert_eq!(speakers, vec![s("me"), s("guest"), s("me"), s("guest")]);
}

#[test]
fn a_cast_is_declared_in_front_matter_too() {
    let p = resolved("---\nvoices:\n  host:\n    voice: bf_emma\n---\n\n# Talk\n\n**Host:** Hi.\n")
        .unwrap();
    assert_eq!(
        voices(&p)[0],
        (s("host"), "kokoro".into(), s("bf_emma"), None)
    );
}

#[test]
fn a_label_naming_nobody_is_read_aloud_with_a_warning() {
    let p = resolved("# Talk\n\n**Gust:** Hi. {#a}\n\n**Note**: this is said.\n").unwrap();
    assert_eq!(texts(&p), ["Gust: Hi.", "Note: this is said."]);
    assert_eq!(voices(&p)[0].0, None);
    let warned: Vec<_> = p.warnings.iter().map(|w| w.message.clone()).collect();
    assert!(
        warned[0].contains("`Gust:`") && warned[0].contains("`guest`"),
        "{warned:?}"
    );
    // Bold that isn't a label is only bold.
    let p = resolved("# Talk\n\n**Really** now: no label.\n").unwrap();
    assert_eq!(texts(&p), ["Really now: no label."]);
    assert!(p.warnings.is_empty());
}

#[test]
fn without_a_cast_a_label_is_only_text() {
    let project = PartialConfig::default();
    let parsed = parse_script("# Talk\n\n**Note:** hi.\n").unwrap();
    let p = resolve(&parsed, "t.md", "en", &project, &project).unwrap();
    assert_eq!(texts(&p), ["Note: hi."]);
    assert!(p.warnings.is_empty());
}

#[test]
fn an_unknown_default_speaker_is_an_error_naming_the_cast() {
    let errors = resolved("# Talk {speaker=gust}\n\nHi. {#a}\n").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.contains("`gust`") && e.contains("guest")),
        "{errors:?}"
    );
}

#[test]
fn a_label_with_nothing_to_say_is_an_error() {
    let errors = resolved("# Talk\n\n**Guest:**\n").unwrap_err();
    assert!(errors[0].contains("nothing to say"), "{errors:?}");
}

#[test]
fn an_at_name_says_how_to_name_a_speaker() {
    let span = SourceSpan {
        line: 1,
        column: 1,
        len: 1,
    };
    for raw in ["#a @guest", "#a speaker=guest"] {
        let (_, d) = LineAttrs::parse(raw, span);
        assert!(
            d.iter()
                .any(|d| d.help.as_deref().unwrap_or("").contains("**Guest:**")),
            "{d:?}"
        );
    }
    let (_, d) = BlockAttrs::parse("scene=mock @guest", span);
    assert!(!d.is_empty());
}

#[test]
fn a_heading_takes_an_id_and_settings() {
    let p = resolved(
        "# The interview {#talk voice.speed=1.2 timing.lead_in_ms=300}\n\nHi.\n\n# Next\n\nBye.\n",
    )
    .unwrap();
    assert_eq!(p.chapters[0].title, "The interview");
    assert_eq!(p.chapters[0].slug, "talk");
    let Element::Narration { id, config, .. } = &p.elements[0] else {
        panic!()
    };
    assert_eq!(id.to_string(), "talk-1");
    assert_eq!(config.voice.speed, 1.2);
    let Element::Narration { config, .. } = &p.elements[1] else {
        panic!()
    };
    assert_eq!(config.voice.speed, 1.0, "a chapter's settings stay in it");
}

#[test]
fn a_heading_setting_that_isnt_one_is_an_error() {
    let errors = resolved("# Talk {voice.sped=1.2}\n\nHi.\n").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.contains("sped") && e.contains("chapter `talk`")),
        "{errors:?}"
    );
}
