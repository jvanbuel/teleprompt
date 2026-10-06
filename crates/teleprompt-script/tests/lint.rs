use teleprompt_core::config::PartialConfig;
use teleprompt_script::lint::lint;
use teleprompt_script::parse::parse_script;
use teleprompt_script::program::resolve;

fn warnings(body: &str) -> Vec<String> {
    let src = format!("# A\n\n{body}\n");
    let s = parse_script(&src).unwrap();
    let p = resolve(
        &s,
        "a.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .unwrap();
    lint(&p).into_iter().map(|d| d.message).collect()
}

#[test]
fn a_plain_script_has_nothing_to_say() {
    assert_eq!(
        warnings("Welcome to Acme. Deploying is one command, and it is fast."),
        Vec::<String>::new()
    );
}

/// Said aloud in one breath, and read as captions, a long sentence loses
/// its listener.
#[test]
fn a_sentence_too_long_to_say_in_one_breath() {
    let long: String = (0..34).map(|i| format!("w{i} ")).collect();
    let w = warnings(&format!("Short one. {long}end."));
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].contains("35-word sentence"), "{}", w[0]);
}

/// A voice spells out what reads as code: file names, paths, URLs, flags,
/// identifiers. (Backticks are gone by then: Markdown reads them as text.)
#[test]
fn words_a_voice_reads_as_code() {
    for word in [
        "tour.md",
        "src/main.rs",
        "https://acme.dev",
        "--force",
        "max_stretch",
        "maxStretch",
    ] {
        let w = warnings(&format!("Now open {word} and look."));
        assert_eq!(w.len(), 1, "{word}: {w:?}");
        assert!(w[0].contains(word.trim_matches('`')), "{word}: {}", w[0]);
    }
    // Ordinary punctuation and words are not code.
    assert_eq!(
        warnings("It's done, e.g. twice: first-class, well-known, and v2. Then 3.5 seconds."),
        Vec::<String>::new()
    );
}

/// A word the script says how to pronounce is no longer a problem.
#[test]
fn a_pronounced_word_is_not_flagged() {
    let src = "---\nvoice:\n  pronounce:\n    tour.md: tour dot M D\n---\n\n# A\n\nNow open tour.md and look.\n";
    let s = parse_script(src).unwrap();
    let p = resolve(
        &s,
        "a.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .unwrap();
    assert!(lint(&p).is_empty());
}

#[test]
fn a_doubled_word_is_a_typo() {
    // "that that" and "had had" are English.
    let w = warnings("Open the the file. He said that that had had its day.");
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].contains("\"the the\""), "{}", w[0]);
}
