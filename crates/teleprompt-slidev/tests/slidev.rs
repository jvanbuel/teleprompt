//! The slides scene: a slide of an existing Slidev deck, at a click step.

use teleprompt_core::{BlockId, SourceSpan};
use teleprompt_plugin::scene::contract::{BlockSource, BodyOrigin, Measured, SceneCompiler, Shot};
use teleprompt_slidev::capture::{range, still_name};
use teleprompt_slidev::scene::{parse, Step};
use teleprompt_slidev::SlidevScene;

fn src(body: &str) -> BlockSource {
    BlockSource {
        scene: "slides".into(),
        body: body.into(),
        origin: BodyOrigin::Inline {
            fence: SourceSpan {
                line: 10,
                column: 1,
                len: 0,
            },
        },
    }
}

fn shots(body: &str) -> Vec<Shot> {
    let v = SlidevScene.validate(&src(body)).expect("valid");
    SlidevScene.shots(&v, &BlockId::from("b")).expect("shots")
}

const fn step(slide: u32, clicks: u32) -> Step {
    Step { slide, clicks }
}

/// A shot names a slide the way Slidev's URLs do.
#[test]
fn a_shot_is_a_slide_at_a_click_step() {
    assert_eq!(parse("3\n").unwrap(), Some(step(3, 0)));
    assert_eq!(parse("3?clicks=2\n").unwrap(), Some(step(3, 2)));
    assert_eq!(parse("# a note\n").unwrap(), None);
}

#[test]
fn what_is_not_a_slide_is_refused() {
    assert!(parse("0").is_err(), "slides count from 1");
    assert!(parse("three").is_err());
    assert!(parse("3?click=2").is_err());
    assert!(parse("3\n4\n").unwrap_err().contains("a paragraph each"));
}

/// `check` points at the line, in the script.
#[test]
fn a_bad_shot_is_refused_where_it_is() {
    let diags = SlidevScene
        .validate(&src("1\n# mark\nnext\n"))
        .expect_err("refused");
    assert_eq!(diags.len(), 1, "{diags:#?}");
    assert_eq!(diags[0].span.expect("located").line, 13);
}

/// Marks split a block like every adapter's do; the compiler then refuses
/// the shots after the first, which have no sentence to last as long as.
#[test]
fn marks_split_a_block_into_one_step_each() {
    let s = shots("2\n# mark\n2?clicks=1\n# mark\n2?clicks=2\n");
    assert_eq!(s.len(), 3);
    assert_eq!(s[2].id, "b#2");
}

/// A comment, or spelling out `?clicks=0`, is the same picture and the
/// same key.
#[test]
fn a_shot_is_keyed_on_the_slide_not_its_spelling() {
    assert_eq!(shots("# intro\n2\n")[0].hash, shots("2?clicks=0\n")[0].hash);
    assert_ne!(shots("2\n")[0].hash, shots("2?clicks=1\n")[0].hash);
}

/// A still is the same picture at any length: no length is claimed, and
/// none is written into the key.
#[test]
fn a_still_states_no_length_and_is_not_re_timed() {
    let s = &shots("2\n")[0];
    assert_eq!(SlidevScene.estimate(s), Measured::Unknown);
    assert_eq!(SlidevScene.retime(s, 5_000), None);
    assert!(!SlidevScene.continues());
}

/// The key covers the deck and what Slidev reads beside it.
#[test]
fn the_inputs_are_the_deck_and_its_folders() {
    let scene = teleprompt_core::config::SceneConfig {
        adapter: "slidev".into(),
        settings: [("deck".to_string(), "talk/slides.md".into())]
            .into_iter()
            .collect(),
    };
    let inputs: Vec<String> = SlidevScene
        .inputs(&scene)
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    assert_eq!(inputs[0], "talk/slides.md");
    assert!(inputs.contains(&"talk/components".to_string()));
    assert!(inputs.contains(&"talk/package-lock.json".to_string()));
}

/// Slidev's own file names for a click step, and one export per session
/// of just the slides it needs.
#[test]
fn the_export_is_of_the_slides_needed() {
    assert_eq!(still_name(step(3, 0)), "003-01.png");
    assert_eq!(still_name(step(12, 2)), "012-03.png");
    assert_eq!(range(&[step(4, 1), step(2, 0), step(4, 0)]), "2,4");
}
