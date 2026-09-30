//! Speakers: a cast of named voices, and `@name` on a line saying who
//! speaks it. A speaker's voice applies over the chapter's and under the
//! line's own attributes; a line without one is the narrator's.

use teleprompt_core::attrs::LineAttrs;
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

/// Each line: its speaker, backend, voice and instruction.
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
        "# Talk\n\nWelcome back. {#intro}\n\nOnly on Fridays. {#friday @guest}\n\n\
         Why? {#why @me}\n\nNobody's watching. {#because @guest voice.instruct=conspiratorial}\n",
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
}

#[test]
fn a_chapter_or_script_names_the_speaker_of_its_lines() {
    let p = resolved(
        "---\nspeaker: me\n---\n\n# Mine\n\nHello. {#a}\n\n# Theirs\n\n```yaml teleprompt\nspeaker: guest\n```\n\n\
         Hi. {#b}\n\nAnd me. {#c @me}\n",
    )
    .unwrap();
    let speakers: Vec<_> = voices(&p).into_iter().map(|v| v.0).collect();
    assert_eq!(speakers, vec![s("me"), s("guest"), s("me")]);
}

#[test]
fn a_cast_is_declared_in_front_matter_too() {
    let p =
        resolved("---\nvoices:\n  host:\n    voice: bf_emma\n---\n\n# Talk\n\nHi. {#a @host}\n")
            .unwrap();
    assert_eq!(
        voices(&p)[0],
        (s("host"), "kokoro".into(), s("bf_emma"), None)
    );
}

#[test]
fn an_unknown_speaker_is_an_error_naming_the_cast() {
    let errors = resolved("# Talk\n\nHi. {#a @gust}\n").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.contains("`gust`") && e.contains("guest")),
        "{errors:?}"
    );
}

#[test]
fn a_speaker_is_written_one_way() {
    let span = SourceSpan {
        line: 1,
        column: 1,
        len: 1,
    };
    let (attrs, d) = LineAttrs::parse("#a @guest", span);
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(attrs.speaker.as_deref(), Some("guest"));
    let (_, d) = LineAttrs::parse("#a speaker=guest", span);
    assert!(
        d.iter()
            .any(|d| d.help.as_deref().unwrap_or("").contains("@guest")),
        "{d:?}"
    );
    let (_, d) = LineAttrs::parse("#a @guest @me", span);
    assert!(d.iter().any(|d| d.message.contains("one speaker")), "{d:?}");
    let (_, d) = LineAttrs::parse("#a @", span);
    assert!(!d.is_empty());
}

#[test]
fn a_block_has_no_speaker() {
    let span = SourceSpan {
        line: 1,
        column: 1,
        len: 1,
    };
    let (_, d) = teleprompt_core::attrs::BlockAttrs::parse("scene=mock @guest", span);
    assert!(d.iter().any(|d| d.message.contains("speaker")), "{d:?}");
}
