//! The media scene: images, clips and title cards, one per shot.

use std::path::Path;

use teleprompt_capture::{Frame, Session, SessionShot};
use teleprompt_core::{BlockId, Hash, SourceSpan};
use teleprompt_media::capture::args;
use teleprompt_media::scene::{parse_line, parse_time, Directive};
use teleprompt_media::MediaScene;
use teleprompt_scene::contract::{BlockSource, BodyOrigin, Measured, SceneCompiler, Shot};

fn src(body: &str) -> BlockSource {
    BlockSource {
        scene: "media".into(),
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
    let v = MediaScene.validate(&src(body)).expect("valid");
    MediaScene.shots(&v, &BlockId::from("b")).expect("shots")
}

#[test]
fn times_are_read_the_ways_people_write_them() {
    assert_eq!(parse_time("12").unwrap(), 12_000);
    assert_eq!(parse_time("12.5s").unwrap(), 12_500);
    assert_eq!(parse_time("250ms").unwrap(), 250);
    assert_eq!(parse_time("0:12").unwrap(), 12_000);
    assert_eq!(parse_time("1:02.5").unwrap(), 62_500);
    assert!(parse_time("soon").is_err());
}

#[test]
fn the_three_directives_are_read() {
    assert_eq!(
        parse_line("image src=arch.png fit=cover").unwrap(),
        Directive::Image {
            src: "arch.png".into(),
            cover: true
        }
    );
    assert_eq!(
        parse_line("clip src=a.mp4 from=0:12 to=0:19").unwrap(),
        Directive::Clip {
            src: "a.mp4".into(),
            from_ms: 12_000,
            to_ms: Some(19_000),
            cover: false
        }
    );
    assert_eq!(
        parse_line("title text=\"Part Two: config\" subtitle=\"It's here\"").unwrap(),
        Directive::Title {
            text: "Part Two: config".into(),
            subtitle: Some("It's here".into())
        }
    );
}

/// Every bad line is reported where it is, with the nearest spelling of
/// a mistyped key.
#[test]
fn what_is_wrong_is_said_where_it_is() {
    let diags = MediaScene
        .validate(&src("image src=a.png fti=cover\n# mark\nclip src=a.mp4 from=5 to=2\n# mark\npicture src=x\n"))
        .expect_err("refused");
    assert_eq!(diags.len(), 3, "{diags:#?}");
    assert_eq!(diags[0].span.unwrap().line, 11);
    assert_eq!(diags[0].help.as_deref(), Some("did you mean `fit`?"));
    assert!(diags[1].message.contains("after `from`"));
    assert!(diags[2].message.contains("not a media directive"));
    assert!(parse_line("image")
        .unwrap_err()
        .message
        .contains("needs `src=`"));
}

#[test]
fn one_directive_per_shot() {
    let diags = MediaScene
        .validate(&src("image src=a.png\nimage src=b.png\n"))
        .expect_err("refused");
    assert!(diags[0].message.contains("one directive per shot"));
}

/// A clip's range states its length; a still states none, and takes its
/// sentence's.
#[test]
fn a_ranged_clip_is_exact_and_a_still_is_not() {
    let s =
        shots("clip src=a.mp4 from=2s to=5.5s\n# mark\nimage src=a.png\n# mark\nclip src=a.mp4\n");
    assert_eq!(MediaScene.estimate(&s[0]), Measured::Exact(3_500));
    assert_eq!(MediaScene.estimate(&s[1]), Measured::Unknown);
    assert_eq!(MediaScene.estimate(&s[2]), Measured::Unknown);
    assert_eq!(
        MediaScene.retime(&s[0], 9_000),
        None,
        "re-timing a clip changes its speed"
    );
    assert!(!MediaScene.continues());
}

mod capture {
    use super::*;

    fn session(settings: &[(&str, &str)]) -> Session {
        Session {
            scene: "media".into(),
            adapter: "media".into(),
            name: None,
            settings: settings
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            shots: Vec::new(),
        }
    }

    fn shot(source: &str, ms: u64) -> SessionShot {
        SessionShot {
            id: "a#0".into(),
            key: Hash::of(source.as_bytes()),
            source: source.into(),
            duration_ms: ms,
            wanted: true,
        }
    }

    const FRAME: Frame = Frame {
        width: 1920,
        height: 1080,
        fps: 30,
    };

    fn argv(source: &str, ms: u64) -> String {
        args(
            &shot(source, ms),
            &session(&[]),
            &FRAME,
            Path::new("media"),
            Path::new("/w"),
            Path::new("/c/out.mp4"),
        )
        .unwrap()
        .join(" ")
    }

    #[test]
    fn a_clip_is_its_range_without_its_sound() {
        let a = argv("clip src=a.mp4 from=0:12 to=0:19", 4_000);
        assert!(a.contains("-ss 12.000 -t 7.000 -i media/a.mp4 -an"), "{a}");
    }

    /// Without `to`, a clip plays from `from` for as long as its sentence.
    #[test]
    fn an_open_clip_plays_for_its_slot() {
        let a = argv("clip src=a.mp4 from=3", 4_250);
        assert!(a.contains("-ss 3.000 -t 4.250"), "{a}");
    }

    #[test]
    fn an_image_is_letterboxed_or_cropped() {
        assert!(argv("image src=a.png", 1)
            .contains("force_original_aspect_ratio=decrease,pad=1920:1080"));
        assert!(argv("image src=a.png fit=cover", 1)
            .contains("force_original_aspect_ratio=increase,crop=1920:1080"));
    }

    /// A title's text is read from a file with expansion off, so quotes,
    /// colons and `%{…}` in it are text, not syntax.
    #[test]
    fn a_title_is_drawn_from_text_files() {
        let a = argv("title text=\"50%: done\"", 1);
        assert!(a.contains("drawtext=textfile="), "{a}");
        assert!(a.contains("expansion=none"), "{a}");
        assert!(!a.contains("50%"), "the text is not in the filter: {a}");
    }
}

/// A shot is keyed on the one file it shows; a title shows none.
#[test]
fn a_shot_names_the_file_it_shows() {
    let scene = teleprompt_core::config::SceneConfig {
        adapter: "media".into(),
        settings: [("dir".to_string(), "assets".into())].into_iter().collect(),
    };
    assert_eq!(
        MediaScene.shot_inputs(&scene, "image src=a/b.png"),
        [std::path::PathBuf::from("assets/a/b.png")]
    );
    assert!(MediaScene
        .shot_inputs(&scene, "title text=\"Hi\"")
        .is_empty());
}
