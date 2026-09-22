//! The browser scene: a Playwright script, read as a Playwright script.
//!
//! The vhs adapter reads a tape — it knows what every command costs, so it
//! can say how long a shot takes and re-write it to last longer. None of
//! that is available here, and pretending otherwise would mean inventing a
//! dialect that looks like Playwright and isn't. A script says what it
//! does, not how long it takes.

use teleprompt_core::SourceSpan;
use teleprompt_playwright::PlaywrightScene;
use teleprompt_scene::contract::{BlockSource, BodyOrigin, Measured, SceneCompiler};

const SPAN: SourceSpan = SourceSpan {
    line: 1,
    column: 1,
    len: 0,
};

fn src(body: &str) -> BlockSource {
    BlockSource {
        scene: "playwright".into(),
        body: body.into(),
        origin: BodyOrigin::Inline { fence: SPAN },
    }
}

fn shots(body: &str) -> Vec<teleprompt_scene::contract::Shot> {
    let v = PlaywrightScene.validate(&src(body)).expect("valid");
    PlaywrightScene.shots(&v, "b").expect("shots")
}

/// A script is arbitrary JavaScript. The adapter does not parse it, so it
/// cannot say what it costs — and `Unknown` is how the contract spells
/// that. The scheduler then gives the shot the length of the sentence over
/// it, which is the behaviour a narrated demo wants anyway.
#[test]
fn a_script_does_not_claim_to_know_its_own_duration() {
    let s = shots("await page.goto('http://localhost:8080');\n");
    assert_eq!(s.len(), 1);
    assert_eq!(PlaywrightScene.estimate(&s[0]), Measured::Unknown);
}

/// The other half of the same fact: an adapter that cannot say how long a
/// shot takes cannot re-write it to take longer.
#[test]
fn a_script_cannot_be_re_timed() {
    let s = shots("await page.click('#run');\n");
    assert_eq!(PlaywrightScene.retime(&s[0], 5_000), None);
}

/// Marks split a block into shots, the same as they do for a tape, and for
/// the same reason: one paragraph of narration per picture. `// mark` is a
/// comment, so the block stays a script somebody could paste into a
/// Playwright test.
#[test]
fn marks_split_a_script_into_one_shot_each() {
    let s = shots(
        "await page.goto('/');\n\
         // mark\n\
         await page.click('#dags');\n\
         // mark\n\
         await page.click('#run');\n",
    );
    assert_eq!(s.len(), 3, "three shots: {s:#?}");
    assert!(s[0].source.contains("goto"));
    assert!(s[1].source.contains("#dags"));
    assert!(s[2].source.contains("#run"));
    assert!(
        !s[1].source.contains("goto"),
        "a shot carries its own lines, not the ones before it"
    );
}

/// A block of nothing is not a shot. Same rule the tape adapter applies to
/// a chunk of only comments.
#[test]
fn a_script_of_only_comments_is_not_a_shot() {
    assert!(shots("// just a note\n\n// and another\n").is_empty());
}

/// Every shot's id is stable and distinct, because a capture key is built
/// from it.
#[test]
fn shots_are_identified_by_block_and_index() {
    let s = shots("await a();\n// mark\nawait b();\n");
    assert_eq!(s[0].id, "b#0");
    assert_eq!(s[1].id, "b#1");
    assert_ne!(s[0].hash, s[1].hash);
}

mod capture {
    use teleprompt_capture::{Frame, Session, SessionShot};
    use teleprompt_core::Hash;
    use teleprompt_playwright::capture::script_for;

    fn shot(id: &str, source: &str, ms: u64) -> SessionShot {
        SessionShot {
            id: id.into(),
            key: Hash::of(id.as_bytes()),
            source: source.into(),
            duration_ms: ms,
            wanted: true,
        }
    }

    fn session(shots: Vec<SessionShot>) -> Session {
        Session {
            scene: "ui".into(),
            adapter: "playwright".into(),
            name: None,
            settings: Default::default(),
            shots,
        }
    }

    fn frame() -> Frame {
        Frame {
            width: 1280,
            height: 720,
            fps: 24,
        }
    }

    /// The author's lines reach Playwright unaltered. teleprompt wraps
    /// them; it does not rewrite them, because it does not parse them.
    #[test]
    fn the_script_carries_the_authors_lines_verbatim() {
        let s = session(vec![shot("a#0", "await page.click('#run');", 2_000)]);
        let js = script_for(&s, &frame(), "/tmp/v");
        assert!(js.contains("await page.click('#run');"), "{js}");
    }

    /// A shot that finishes early holds the page until its slot is over,
    /// so the reel's clock matches the windows cut from it. This is not
    /// re-timing: the script is untouched, the wait comes after it.
    #[test]
    fn a_shot_is_held_open_for_the_slot_the_narration_gave_it() {
        let s = session(vec![shot("a#0", "await page.click('#x');", 2_500)]);
        let js = script_for(&s, &frame(), "/tmp/v");
        assert!(
            js.contains("const __left = 2500 - (Date.now() - __began);"),
            "{js}"
        );
        assert!(js.contains("if (__left > 0) await page.waitForTimeout(__left);"));
    }

    /// Recording with a visible cursor is the whole reason to record a
    /// browser: a click with no pointer on screen is a page that changes
    /// for no visible reason.
    #[test]
    fn the_recording_shows_the_cursor_and_what_it_did() {
        let js = script_for(
            &session(vec![shot("a#0", "await page.click('#x');", 100)]),
            &frame(),
            "/tmp/v",
        );
        assert!(js.contains("showActions"), "{js}");
        assert!(js.contains("cursor: 'pointer'"), "{js}");
    }

    /// Each shot runs in its own block, in order, in one context — the
    /// page keeps its state across them.
    #[test]
    fn the_shots_run_in_order_in_one_browser() {
        let s = session(vec![
            shot("a#0", "await page.goto('/');", 1_000),
            shot("a#1", "await page.click('#next');", 1_000),
        ]);
        let js = script_for(&s, &frame(), "/tmp/v");
        assert_eq!(js.matches("newContext").count(), 1, "one context: {js}");
        assert!(js.find("goto").unwrap() < js.find("#next").unwrap());
    }
}
