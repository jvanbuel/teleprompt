use teleprompt_core::config::PartialConfig;
use teleprompt_core::ident::assign_ids;
use teleprompt_core::parse::parse_script;
use teleprompt_core::policy::PolicyKind;
use teleprompt_core::program::{resolve, Element};
use teleprompt_derive::{derive, Options, Trace, Word};
use teleprompt_vhs::scene::classify;

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

fn session() -> (Trace, Vec<Word>) {
    let mut trace = Trace::default();
    for (i, c) in "clear\r".chars().enumerate() {
        trace.input.push((i as u64 * 60, c.to_string()));
    }
    for (i, c) in "ls -la\r".chars().enumerate() {
        trace.input.push((1850 + i as u64 * 60, c.to_string()));
    }
    for (i, c) in "cat \"a b\".txt\r".chars().enumerate() {
        trace.input.push((30_000 + i as u64 * 60, c.to_string()));
    }
    trace.output = vec![2400, 30_900];
    let words = [
        say(1000, "LET'S SEE WHAT IS HERE"),
        say(
            6000,
            "AND NOW THE FILE WHICH I WROTE EARLIER TODAY SO THAT THIS LINE RUNS LONGER \
             THAN ONE LINE OF A SCRIPT IS WIDE",
        ),
    ]
    .concat();
    (trace, words)
}

/// What `derive` writes is a script: it parses and resolves, its policies
/// and cues are ones the compiler accepts, and its tapes are tapes the
/// terminal scene reads.
#[test]
fn a_derived_script_is_a_script() {
    let (trace, words) = session();
    let options = Options {
        title: "Tour".to_string(),
        ..Options::default()
    };
    let md = derive(&trace, &words, &options).markdown(&options);

    let mut script = parse_script(&md).unwrap();
    assign_ids(&mut script);
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
            Element::Action {
                body, policy, cue, ..
            } => {
                for line in body.lines() {
                    classify(line).unwrap_or_else(|e| panic!("{line}: {e:?}"));
                }
                actions.push((*policy, cue.clone()));
            }
            _ => {}
        }
    }
    let long = "And now the file which I wrote earlier today so that this line runs \
                longer than one line of a script is wide.";
    assert_eq!(narration, ["Let's see what is here.", long]);
    assert!(md.lines().all(|l| l.len() <= 76), "wrapped:\n{md}");
    assert_eq!(
        actions,
        [
            (PolicyKind::Hold, None),
            (PolicyKind::Concurrent, Some("what is".to_string())),
            (PolicyKind::Hold, None),
        ]
    );
    assert!(md.contains("# Tour\n"), "{md}");
}
