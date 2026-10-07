//! The motion scene: a composition from an existing Remotion project,
//! rendered at the length the narration gives it.

use teleprompt_scene::contract::{BlockSource, BodyOrigin, Measured, SceneCompiler, Shot};
use teleprompt_scene::core::{BlockId, SourceSpan};
use teleprompt_scenes::remotion::scene::parse;
use teleprompt_scenes::remotion::RemotionScene;

fn src(body: &str) -> BlockSource {
    BlockSource {
        scene: "motion".into(),
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
    let v = RemotionScene.validate(&src(body)).expect("valid");
    RemotionScene.shots(&v, &BlockId::from("b")).expect("shots")
}

/// A shot is what `remotion render` takes: a composition id and props.
#[test]
fn a_shot_names_a_composition_and_its_props() {
    let call = parse("Title {\"title\": \"Hi\"}\n").unwrap().unwrap();
    assert_eq!(call.composition, "Title");
    assert_eq!(call.props, serde_json::json!({"title": "Hi"}));

    let bare = parse("Outro\n").unwrap().unwrap();
    assert_eq!(bare.props, serde_json::json!({}), "props are optional");
}

/// Props may span lines, and `#` lines are comments.
#[test]
fn props_may_span_lines() {
    let call = parse("# the opening card\nTitle {\n  \"title\": \"Hi\"\n}\n")
        .unwrap()
        .unwrap();
    assert_eq!(call.props["title"], "Hi");
}

/// Marks split a block into shots, one composition each.
#[test]
fn marks_split_a_block_into_one_shot_each() {
    let s = shots("Title {\"title\": \"One\"}\n# mark\nCaption\n# mark\n# only a note\n");
    assert_eq!(s.len(), 2, "a chunk of only comments is not a shot: {s:#?}");
    assert!(s[0].source.contains("One"));
    assert!(s[1].source.contains("Caption"));
    assert_eq!(s[1].id, "b#1");
}

/// Props that are not JSON are refused at `check`, on the line that names
/// the composition — which is also what two compositions in one block look
/// like, so the message says how to separate them.
#[test]
fn props_that_are_not_json_are_refused_where_they_are() {
    let diags = RemotionScene
        .validate(&src("Title\n# mark\nTitle {title: 1}\n"))
        .expect_err("refused");
    assert_eq!(diags.len(), 1, "{diags:#?}");
    assert_eq!(diags[0].span.expect("located").line, 13);
    assert!(diags[0].message.contains("not JSON"), "{diags:#?}");
    assert!(diags[0].message.contains("own paragraph"), "{diags:#?}");
}

#[test]
fn props_must_be_an_object_and_ids_must_be_ids() {
    assert!(parse("Title [1, 2]").is_err());
    assert!(
        parse("my_title").is_err(),
        "Remotion ids have no underscores"
    );
}

/// The block states no length; the scheduler gives the shot its sentence.
#[test]
fn a_composition_does_not_claim_a_length_of_its_own() {
    assert_eq!(shots("Title\n")[0].length, Measured::Unknown);
}

/// Re-timing always succeeds and puts the length into the source, which
/// is what the capture key is built from — once, however often it is done.
#[test]
fn a_shot_is_re_timed_by_stating_its_length_once() {
    let s = &shots("Title\n")[0];
    let four = RemotionScene.retime(s, 4_000).expect("always re-timable");
    assert!(four.starts_with("# teleprompt: 4000ms\n"), "{four}");
    let again = RemotionScene
        .retime(
            &Shot {
                source: four.clone(),
                ..s.clone()
            },
            5_000,
        )
        .unwrap();
    assert_eq!(again.trim_end(), "# teleprompt: 5000ms\nTitle");
    assert_eq!(parse(&again).unwrap().unwrap().composition, "Title");
}

/// The files the key covers are what Remotion's bundler reads, by its
/// own layout — not the whole project, which defaults to `.`.
#[test]
fn the_inputs_are_what_the_bundler_reads() {
    let scene: teleprompt_scene::core::config::SceneConfig =
        teleprompt_scene::core::config::SceneConfig {
            plugin: "remotion".into(),
            settings: [
                ("project".to_string(), "motion".into()),
                ("entry".to_string(), "app/main.ts".into()),
            ]
            .into_iter()
            .collect(),
            root: Default::default(),
        };
    let inputs = RemotionScene.inputs(&scene);
    let names: Vec<String> = inputs.iter().map(|p| p.display().to_string()).collect();
    assert_eq!(
        names,
        [
            "motion/app",
            "motion/public",
            "motion/package.json",
            "motion/package-lock.json",
            "motion/remotion.config.ts",
        ]
    );
}

/// A composition does not open on the screen before it, so the compiler
/// names each shot by itself.
#[test]
fn a_shot_does_not_continue_the_one_before_it() {
    assert!(!RemotionScene.continues());
}

mod capture {
    use std::path::Path;

    use teleprompt_scene::capture::{Frame, Session, SessionShot};
    use teleprompt_scene::core::Hash;
    use teleprompt_scenes::remotion::capture::{frames, job_for};

    fn shot(id: &str, source: &str, ms: u64, wanted: bool) -> SessionShot {
        SessionShot {
            id: id.into(),
            key: Hash::of(id.as_bytes()),
            source: source.into(),
            duration_ms: ms,
            wanted,
        }
    }

    fn job(shots: Vec<SessionShot>) -> serde_json::Value {
        let session = Session {
            scene: "motion".into(),
            plugin: "remotion".into(),
            name: None,
            settings: Default::default(),
            root: Default::default(),
            shots,
        };
        let frame = Frame {
            width: 1920,
            height: 1080,
            fps: 30,
        };
        job_for(
            &session,
            &frame,
            Path::new("/p/src/index.ts"),
            Path::new("/c"),
        )
        .unwrap()
    }

    #[test]
    fn frames_round_to_the_nearest_and_never_to_none() {
        assert_eq!(frames(1_000, 30), 30);
        assert_eq!(frames(1_020, 30), 31);
        assert_eq!(frames(1_010, 30), 30);
        assert_eq!(frames(0, 30), 1);
    }

    /// Each wanted shot is rendered from the project's own entry, at its
    /// slot's length, straight to the clip filed under its capture key.
    #[test]
    fn each_shot_is_rendered_at_its_slot_to_its_clip() {
        let j = job(vec![shot(
            "a#0",
            "# teleprompt: 2500ms\nTitle {\"title\": \"Hi\"}\n",
            2_500,
            true,
        )]);
        assert_eq!(j["entry"], "/p/src/index.ts");
        let s = &j["shots"][0];
        assert_eq!(s["composition"], "Title");
        assert_eq!(s["props"]["title"], "Hi");
        assert_eq!(s["frames"], 75);
        assert_eq!(s["out"], format!("/c/{}.mp4", Hash::of(b"a#0")));
    }

    /// A cached shot is not rendered: there is no screen to reach.
    #[test]
    fn a_cached_shot_is_not_rendered() {
        let j = job(vec![
            shot("a#0", "Title", 1_000, false),
            shot("a#1", "Caption", 1_000, true),
        ]);
        assert_eq!(j["shots"].as_array().unwrap().len(), 1);
        assert_eq!(j["shots"][0]["composition"], "Caption");
    }
}
