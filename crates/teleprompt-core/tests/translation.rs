use teleprompt_core::config::PartialConfig;
use teleprompt_core::ident::assign_ids;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::{resolve, Element, Program};
use teleprompt_core::translation::{apply, source_of, Entry, Translation};
use teleprompt_core::{Hash, Severity};

const SRC: &str = r#"# Introduction

Welcome to Acme. {#welcome}

Run flowrs config add to start. {#start}

```teleprompt scene=mock policy=concurrent cue="config add"
wait 500ms
```

Then it streams its progress. {#progress}

```teleprompt scene=mock policy=concurrent cue="streams"
wait 500ms
```
"#;

fn program(locale: &str) -> Program {
    let mut s = parse_script(SRC).unwrap();
    assign_ids(&mut s);
    resolve(
        &s,
        "tour.md",
        locale,
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .unwrap()
}

fn entry(english: &str, text: &str) -> Entry {
    Entry {
        from: source_of(english),
        text: text.to_string(),
    }
}

fn dutch() -> Translation {
    let mut t = Translation::default();
    t.chapters
        .push(("introduction".into(), entry("Introduction", "Inleiding")));
    t.lines.push((
        "welcome".into(),
        entry("Welcome to Acme.", "Welkom bij Acme."),
    ));
    t.lines.push((
        "start".into(),
        entry(
            "Run flowrs config add to start.",
            "Voer flowrs config add uit om te beginnen.",
        ),
    ));
    t.lines.push((
        "progress".into(),
        entry(
            "Then it streams its progress.",
            "Daarna toont het de voortgang.",
        ),
    ));
    t
}

fn narration(p: &Program) -> Vec<(&str, &str)> {
    p.elements
        .iter()
        .filter_map(|e| match e {
            Element::Narration { id, text, .. } => Some((id.as_str(), text.as_str())),
            _ => None,
        })
        .collect()
}

fn cues(p: &Program) -> Vec<Option<&str>> {
    p.elements
        .iter()
        .filter_map(|e| match e {
            Element::Action { cue, .. } => Some(cue.as_deref()),
            _ => None,
        })
        .collect()
}

/// The translated lines are what is said, and hash as themselves, so their
/// audio is never taken for the English line's.
#[test]
fn translated_lines_replace_the_narration() {
    let mut p = program("nl");
    let diags = apply(&mut p, &dutch());
    assert!(
        diags.iter().all(|d| d.severity != Severity::Error),
        "{diags:?}"
    );
    assert_eq!(
        narration(&p),
        [
            ("welcome", "Welkom bij Acme."),
            ("start", "Voer flowrs config add uit om te beginnen."),
            ("progress", "Daarna toont het de voortgang."),
        ]
    );
    let hashes: Vec<Hash> = p
        .elements
        .iter()
        .filter_map(|e| match e {
            Element::Narration {
                source_hash, text, ..
            } => {
                assert_eq!(*source_hash, Hash::of(text.as_bytes()));
                Some(*source_hash)
            }
            _ => None,
        })
        .collect();
    assert_eq!(hashes.len(), 3);
    assert_eq!(p.chapters[0].title, "Inleiding");
}

/// A line with no translation cannot be spoken in the locale.
#[test]
fn a_missing_line_is_an_error_naming_the_command() {
    let mut p = program("nl");
    let mut t = dutch();
    t.lines.retain(|(id, _)| id != "progress");
    let diags = apply(&mut p, &t);
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    assert_eq!(errors.len(), 1, "{diags:?}");
    assert!(
        errors[0].message.contains("`progress`"),
        "{}",
        errors[0].message
    );
    assert!(errors[0]
        .help
        .as_deref()
        .unwrap()
        .contains("teleprompt translate"));
}

/// A translation of English that has since changed still plays, and says
/// it is out of date.
#[test]
fn a_stale_line_is_a_warning() {
    let mut p = program("nl");
    let mut t = dutch();
    t.lines[0].1 = entry("Welcome to Acme Corp.", "Welkom bij Acme Corp.");
    let diags = apply(&mut p, &t);
    assert_eq!(narration(&p)[0].1, "Welkom bij Acme Corp.");
    let stale: Vec<_> = diags
        .iter()
        .filter(|d| d.message.contains("changed"))
        .collect();
    assert_eq!(stale.len(), 1, "{diags:?}");
    assert_eq!(stale[0].severity, Severity::Warning);
    assert!(
        stale[0].message.contains("line `welcome`"),
        "{}",
        stale[0].message
    );
}

/// A cue is looked for in the translation: as the sidecar gives it, as
/// written when the translation kept it (a command), or else at the same
/// place in the line, with a warning.
#[test]
fn cues_follow_the_translation() {
    let mut p = program("nl");
    let diags = apply(&mut p, &dutch());
    let c = cues(&p);
    assert_eq!(c[0], Some("config add"), "kept verbatim");
    let second = c[1].unwrap();
    assert!(
        "Daarna toont het de voortgang.".contains(second),
        "{second}"
    );
    assert!(
        diags
            .iter()
            .any(|d| d.severity == Severity::Warning && d.message.contains("streams")),
        "{diags:?}"
    );

    let mut p = program("nl");
    let mut t = dutch();
    let block = match &p.elements[4] {
        Element::Action { block_id, .. } => block_id.clone(),
        other => panic!("{other:?}"),
    };
    t.cues.push((block, entry("streams", "toont")));
    let diags = apply(&mut p, &t);
    assert_eq!(cues(&p)[1], Some("toont"));
    assert!(diags.is_empty(), "{diags:?}");
}

/// The sidecar reads and writes as YAML, in the order it was given.
#[test]
fn a_translation_round_trips_through_yaml_in_order() {
    let t = dutch();
    let yaml = t.to_yaml("tour.md", "nl");
    assert!(
        yaml.find("welcome:").unwrap() < yaml.find("start:").unwrap(),
        "{yaml}"
    );
    assert!(
        yaml.starts_with('#'),
        "a header says what the file is:\n{yaml}"
    );
    let back = Translation::from_yaml(&yaml).unwrap();
    assert_eq!(back, t);
}
