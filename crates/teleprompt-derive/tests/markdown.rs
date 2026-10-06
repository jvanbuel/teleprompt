use teleprompt_core::config::PartialConfig;
use teleprompt_core::policy::PolicyKind;
use teleprompt_derive::{derive, Options, Word};
use teleprompt_script::parse::parse_script;
use teleprompt_script::program::{resolve, ActionElement, Element};

fn say(at: u64, text: &str) -> Vec<Word> {
    text.split_whitespace()
        .enumerate()
        .map(|(i, w)| Word {
            text: w.to_string(),
            start_ms: at + i as u64 * 400,
            end_ms: at + i as u64 * 400 + 350,
        })
        .collect()
}

/// Steps at 0 and 1.85 s, before and during the first line, and one at
/// 30 s after the second.
fn session() -> (Vec<u64>, Vec<Word>) {
    let starts = vec![0, 1850, 30_000];
    let words = [
        say(1000, "LET'S SEE WHAT IS HERE"),
        say(
            6000,
            "AND NOW THE FILE WHICH I WROTE EARLIER TODAY SO THAT THIS LINE RUNS LONGER \
             THAN ONE LINE OF A SCRIPT IS WIDE",
        ),
    ]
    .concat();
    (starts, words)
}

/// What `derive` writes is a script: it parses and resolves, its policies
/// and cues are ones the compiler accepts, and each block includes its
/// part of the recording.
#[test]
fn a_derived_script_is_a_script() {
    let (starts, words) = session();
    let options = Options {
        title: "Tour".to_string(),
        include: "recordings/tour.cast".to_string(),
        ..Options::default()
    };
    let md = derive(&starts, &words, &options).markdown(&options);

    let script = parse_script(&md).unwrap();
    let program = resolve(
        &script,
        "tour.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .unwrap_or_else(|e| panic!("{md}\n{:?}", e.0));

    let mut narration = Vec::new();
    let mut actions = Vec::new();
    for element in &program.elements {
        match element {
            Element::Narration { text, .. } => narration.push(text.clone()),
            Element::Action(ActionElement {
                body,
                policy,
                cue,
                include,
                scene,
                ..
            }) => {
                assert!(body.trim().is_empty(), "{body}");
                assert_eq!(scene, "asciinema");
                actions.push((include.clone().unwrap(), *policy, cue.clone()));
            }
            _ => {}
        }
    }
    let long = "And now the file which I wrote earlier today so that this line runs \
                longer than one line of a script is wide.";
    assert_eq!(narration, ["Let's see what is here.", long]);
    // Prose is wrapped; a fence's attributes cannot be.
    assert!(
        md.lines()
            .filter(|l| !l.starts_with("```"))
            .all(|l| l.len() <= 76),
        "wrapped:\n{md}"
    );
    assert_eq!(
        actions,
        [
            ("recordings/tour.cast#1".to_string(), PolicyKind::Hold, None),
            (
                "recordings/tour.cast#2".to_string(),
                PolicyKind::Concurrent,
                Some("what is".to_string())
            ),
            ("recordings/tour.cast#3".to_string(), PolicyKind::Hold, None),
        ]
    );
    assert!(md.contains("# Tour\n"), "{md}");
}
