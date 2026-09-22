//! The motion scene: JSX, rendered by Remotion at the length it is given.
//!
//! The tape adapter knows what a tape costs; the Playwright adapter knows
//! a script costs whatever the page takes. This one knows a composition
//! costs whatever it is told — so it states no length, and re-writing a
//! shot to any length always succeeds.

use teleprompt_core::SourceSpan;
use teleprompt_remotion::RemotionScene;
use teleprompt_scene::contract::{BlockSource, BodyOrigin, Measured, SceneCompiler, Shot};

const SPAN: SourceSpan = SourceSpan {
    line: 10,
    column: 1,
    len: 0,
};

fn src(body: &str) -> BlockSource {
    BlockSource {
        scene: "motion".into(),
        body: body.into(),
        origin: BodyOrigin::Inline { fence: SPAN },
    }
}

fn shots(body: &str) -> Vec<Shot> {
    let v = RemotionScene.validate(&src(body)).expect("valid");
    RemotionScene.shots(&v, "b").expect("shots")
}

/// The block states no length, so the scheduler gives the shot the
/// sentence spoken over it.
#[test]
fn a_composition_does_not_claim_a_length_of_its_own() {
    let s = shots("<Title title=\"Hi\" />\n");
    assert_eq!(s.len(), 1);
    assert_eq!(RemotionScene.estimate(&s[0]), Measured::Unknown);
}

/// Marks split a block into shots, one per sentence, and the mark is a
/// JSX comment so the block stays JSX.
#[test]
fn marks_split_a_block_into_one_shot_each() {
    let s = shots(
        "<Title title=\"One\" />\n\
         {/* mark */}\n\
         <Title title=\"Two\" />\n\
         {/* mark */}\n\
         <Title title=\"Three\" />\n",
    );
    assert_eq!(s.len(), 3, "{s:#?}");
    assert!(s[0].source.contains("One"));
    assert!(s[1].source.contains("Two") && !s[1].source.contains("One"));
    assert!(s[2].source.contains("Three"));
    assert_eq!(s[0].id, "b#0");
    assert_eq!(s[2].id, "b#2");
}

/// A chunk of only comments draws nothing, and is not a shot.
#[test]
fn a_chunk_of_only_comments_is_not_a_shot() {
    assert!(shots("{/* a note */}\n\n{/* another */}\n").is_empty());
    assert_eq!(shots("{/* a note */} <Title />\n").len(), 1);
}

/// `// mark` is how the Playwright adapter spells a mark. In JSX children
/// it is text, and would be drawn — so it is refused, at the line it is on,
/// with the spelling that works.
#[test]
fn a_mark_spelled_as_a_line_comment_is_refused() {
    let diags = RemotionScene
        .validate(&src("<Title />\n// mark\n<Title />\n"))
        .expect_err("refused");
    assert_eq!(diags.len(), 1, "{diags:#?}");
    assert_eq!(diags[0].span.expect("located").line, 12);
    assert!(diags[0].message.contains("drawn on screen"), "{diags:#?}");
    assert!(
        diags[0]
            .help
            .as_deref()
            .is_some_and(|h| h.contains("{/* mark */}")),
        "{diags:#?}"
    );
}

/// Re-timing always succeeds, and puts the length into the source — which
/// is what the capture key is built from, so a shot rendered for four
/// seconds is not reused for six.
#[test]
fn a_shot_is_re_timed_by_stating_its_length_in_its_source() {
    let s = &shots("<Title />\n")[0];
    let four = RemotionScene.retime(s, 4_000).expect("always re-timable");
    let six = RemotionScene.retime(s, 6_000).expect("always re-timable");
    assert!(four.starts_with("{/* teleprompt: 4000ms */}\n"), "{four}");
    assert!(four.trim_end().ends_with("<Title />"), "{four}");
    assert_ne!(four, six);
}

/// Re-timing a re-timed shot states one length, not two.
#[test]
fn re_timing_twice_states_one_length() {
    let s = &shots("<Title />\n")[0];
    let once = RemotionScene.retime(s, 4_000).expect("re-timed");
    let again = RemotionScene
        .retime(
            &Shot {
                source: once,
                ..s.clone()
            },
            5_000,
        )
        .expect("re-timed");
    assert_eq!(again.trim_end(), "{/* teleprompt: 5000ms */}\n<Title />");
}

/// A composition is a function of its own frame, so a shot does not open
/// on the one before it — and the compiler is told so, and names each
/// shot by itself.
#[test]
fn a_shot_does_not_continue_the_one_before_it() {
    assert!(!RemotionScene.continues());
}

mod capture {
    use std::path::Path;

    use teleprompt_capture::{Frame, Session, SessionShot};
    use teleprompt_core::Hash;
    use teleprompt_remotion::capture::{entry_for, frames, render_script_for};

    fn shot(id: &str, source: &str, ms: u64, wanted: bool) -> SessionShot {
        SessionShot {
            id: id.into(),
            key: Hash::of(id.as_bytes()),
            source: source.into(),
            duration_ms: ms,
            wanted,
        }
    }

    fn session(shots: Vec<SessionShot>) -> Session {
        Session {
            scene: "motion".into(),
            adapter: "remotion".into(),
            name: None,
            settings: Default::default(),
            shots,
        }
    }

    const FRAME: Frame = Frame {
        width: 1920,
        height: 1080,
        fps: 30,
    };

    /// The composition is exactly as long as the slot: the clip Remotion
    /// writes is the clip the renderer places, with nothing to cut.
    #[test]
    fn each_shot_is_a_composition_as_long_as_its_slot() {
        let s = session(vec![shot("a#0", "<Title />", 2_500, true)]);
        let jsx = entry_for(&s, &FRAME, None);
        assert!(
            jsx.contains("durationInFrames={75} fps={30} width={1920} height={1080}"),
            "{jsx}"
        );
    }

    /// Frames are rounded, not truncated, and a shot always has one.
    #[test]
    fn frames_round_to_the_nearest_and_never_to_none() {
        assert_eq!(frames(1_000, 30), 30);
        assert_eq!(frames(1_020, 30), 31);
        assert_eq!(frames(1_010, 30), 30);
        assert_eq!(frames(0, 30), 1);
    }

    /// The author's JSX reaches Remotion unaltered, inside a full frame.
    #[test]
    fn the_entry_carries_the_authors_jsx_verbatim() {
        let s = session(vec![shot(
            "a#0",
            "<Title title=\"It's here\" />\n<Pipeline steps={[\"a\"]} />",
            1_000,
            true,
        )]);
        let jsx = entry_for(&s, &FRAME, Some("/p/src/components.tsx"));
        assert!(jsx.contains("<Title title=\"It's here\" />"), "{jsx}");
        assert!(jsx.contains("<Pipeline steps={[\"a\"]} />"), "{jsx}");
        assert!(
            jsx.contains("import * as __components from \"/p/src/components.tsx\";"),
            "{jsx}"
        );
        assert!(jsx.contains("registerRoot(Root);"), "{jsx}");
    }

    /// The components a shot names are brought into scope by name, from
    /// the project first and Remotion second — and `props.Title` is a
    /// property, not a name.
    #[test]
    fn the_components_a_shot_names_are_in_scope() {
        let s = session(vec![shot(
            "a#0",
            "<AbsoluteFill><Title text={props.Subtitle} /></AbsoluteFill>",
            1_000,
            true,
        )]);
        let jsx = entry_for(&s, &FRAME, None);
        assert!(
            jsx.contains("const Title = __scope[\"Title\"] ?? globalThis[\"Title\"];"),
            "{jsx}"
        );
        assert!(jsx.contains("const AbsoluteFill = __scope[\"AbsoluteFill\"]"));
        assert!(!jsx.contains("const Subtitle"), "{jsx}");
    }

    /// A shot whose clip is cached is not rendered: nothing carries over
    /// from one composition to the next, so there is no screen to reach.
    #[test]
    fn a_cached_shot_is_not_rendered() {
        let s = session(vec![
            shot("a#0", "<Title title=\"cached\" />", 1_000, false),
            shot("a#1", "<Title title=\"wanted\" />", 1_000, true),
        ]);
        let jsx = entry_for(&s, &FRAME, None);
        assert!(!jsx.contains("cached"), "{jsx}");
        assert_eq!(jsx.matches("<Composition").count(), 1, "{jsx}");

        let js = render_script_for(
            &s,
            Path::new("/w/index.jsx"),
            Path::new("/p"),
            Path::new("/c"),
        );
        assert!(js.contains("\"shot\":\"a#1\""), "{js}");
        assert!(!js.contains("a#0"), "{js}");
    }

    /// Each clip is written where the build looks for it: under its
    /// capture key, in the clips directory.
    #[test]
    fn each_clip_is_filed_under_its_capture_key() {
        let s = session(vec![shot("a#0", "<Title />", 1_000, true)]);
        let key = Hash::of(b"a#0");
        let js = render_script_for(
            &s,
            Path::new("/w/index.jsx"),
            Path::new("/p"),
            Path::new("/c"),
        );
        assert!(js.contains(&format!("\"/c/{key}.mp4\"")), "{js}");
        assert!(js.contains("bundle({ entryPoint: \"/w/index.jsx\", rootDir: \"/p\" })"));
    }

    /// A browser named by the scene is the one Remotion renders with.
    #[test]
    fn a_scene_may_name_its_browser() {
        let mut s = session(vec![shot("a#0", "<Title />", 1_000, true)]);
        s.settings
            .insert("browser".into(), "/opt/chrome/headless_shell".into());
        let js = render_script_for(
            &s,
            Path::new("/w/index.jsx"),
            Path::new("/p"),
            Path::new("/c"),
        );
        assert!(
            js.contains("const browserExecutable = \"/opt/chrome/headless_shell\";"),
            "{js}"
        );
    }
}
