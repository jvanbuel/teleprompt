//! A prompter session driven directly, with a recognizer that hears what
//! the test says.

use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;

use teleprompt_core::Hash;
use teleprompt_listen::{Heard, Recognizer};
use teleprompt_serve::prompter::{Position, Prompt, Reached, Session, ShotCue};

/// Hears what the test says, one hypothesis per chunk, and counts the
/// samples it was given.
struct Scripted(VecDeque<&'static str>, Rc<Cell<usize>>);

impl Recognizer for Scripted {
    fn listen(&mut self, samples: &[f32]) -> Heard {
        self.1.set(self.1.get() + samples.len());
        Heard {
            text: self.0.pop_front().unwrap_or_default().to_string(),
            is_final: false,
        }
    }

    fn reset(&mut self) {}
}

const LINES: &[&str] = &[
    "Welcome to Acme. Let me show you around.",
    "Deployment is one command.",
];

fn pos(line: usize, word: usize) -> Position {
    Position { line, word }
}

/// A shot before the first line, one three words in, and one after the
/// first line; only the second has been captured.
fn prompt(dir: &std::path::Path) -> Prompt {
    let cue = |shot: &str, at| ShotCue {
        shot: shot.into(),
        capture_key: Hash::of(shot.as_bytes()),
        at,
    };
    std::fs::write(
        dir.join(format!("{}.mp4", Hash::of(b"welcome-a#0"))),
        b"clip",
    )
    .unwrap();
    Prompt {
        name: "tour.md".into(),
        lines: LINES.iter().map(|l| l.to_string()).collect(),
        ids: vec!["welcome".into(), "deploy".into()],
        shots: vec![
            cue("intro#0", pos(0, 0)),
            cue("welcome-a#0", pos(0, 3)),
            cue("welcome-b#0", pos(1, 0)),
        ],
        clips: teleprompt::project::CacheDir::at(dir),
        takes: dir.join("takes"),
    }
}

struct Fixture {
    session: Session<Scripted>,
    heard: Rc<Cell<usize>>,
    dir: teleprompt_testkit::TestDir,
}

fn session(tag: &str, heard: &[&'static str]) -> Fixture {
    let dir = teleprompt_testkit::test_dir(tag);
    let count = Rc::new(Cell::new(0));
    let recognizer = Scripted(heard.iter().copied().collect(), count.clone());
    Fixture {
        session: Session::new(prompt(&dir), recognizer).unwrap(),
        heard: count,
        dir,
    }
}

fn silence(n: usize) -> Vec<f32> {
    vec![0.0; n]
}

fn plays(shots: &[&str]) -> Vec<teleprompt_core::ShotId> {
    shots.iter().map(|s| (*s).into()).collect()
}

/// Reaching a shot's cue says to play it, once.
#[test]
fn reaching_a_shot_says_to_play_it_once() {
    let mut f = session("prompter-once", &["welcome to acme", "welcome to acme let"]);
    let first = f.session.listen(&silence(1600), 16_000);
    assert_eq!(first.play, plays(&["intro#0", "welcome-a#0"]));
    assert_eq!(f.session.listen(&silence(1600), 16_000).play, plays(&[]));
}

/// Starting a take goes back to the top, whatever the last one reached,
/// and the shot before the first line plays again.
#[test]
fn starting_a_take_goes_back_to_the_top() {
    let mut f = session("prompter-top", &["welcome to acme"]);
    f.session.listen(&silence(1600), 16_000);
    assert_eq!(
        f.session.start(0),
        Reached {
            at: pos(0, 0),
            play: plays(&["intro#0"])
        }
    );
}

/// A take can start at a line, to read it again; the shots before it do
/// not play.
#[test]
fn a_take_can_start_at_a_line() {
    let mut f = session("prompter-from", &["deployment is"]);
    assert_eq!(
        f.session.start(1),
        Reached {
            at: pos(1, 0),
            play: plays(&["welcome-b#0"])
        }
    );
    assert_eq!(f.session.listen(&silence(1600), 16_000).at, pos(1, 2));
}

/// Audio at the microphone's rate reaches the recognizer at its own.
#[test]
fn audio_at_another_rate_is_heard_at_the_recognizers() {
    let mut f = session("prompter-rate", &[]);
    f.session.listen(&silence(4_800), 48_000);
    f.session.listen(&silence(4_800), 48_000);
    let n = f.heard.get();
    assert!((3_100..=3_200).contains(&n), "{n} samples at 16 kHz");
}

/// 16 kHz audio: `spans` of (seconds, loud).
fn audio(spans: &[(f32, bool)]) -> Vec<f32> {
    spans
        .iter()
        .flat_map(|&(seconds, loud)| {
            (0..(seconds * 16_000.0) as usize).map(move |i| {
                if loud {
                    0.3 * (i as f32 * 0.2).sin()
                } else {
                    0.0
                }
            })
        })
        .collect()
}

/// A take's lines read in full are kept as takes, each with the text it
/// was read from; a line broken off is not.
#[test]
fn stopping_a_take_keeps_each_line_read_in_full() {
    const LINE_0: &str = "welcome to acme let me show you around";
    let mut heard = vec![""; 5];
    heard.extend(["welcome to"; 6]);
    heard.extend([LINE_0; 7]);
    heard.extend([""; 5]);
    heard.extend(["deployment is"; 7]);
    let mut f = session("prompter-stop", &heard);
    f.session.start(0);
    let take = audio(&[(0.3, false), (1.2, true), (0.8, false), (0.7, true)]);
    for chunk in take.chunks(1_600) {
        f.session.listen(chunk, 16_000);
    }
    assert_eq!(f.session.stop().unwrap(), ["welcome"]);

    let takes = teleprompt_voice::takes::Takes::load(&f.dir.join("takes")).unwrap();
    let welcome = takes
        .current("welcome", LINES[0])
        .expect("a take of line 0");
    assert!(
        (1_200..=1_320).contains(&welcome.duration_ms),
        "{}",
        welcome.duration_ms
    );
    assert!(takes.current("deploy", LINES[1]).is_none());

    let recorded: Vec<bool> = f
        .session
        .script()
        .lines
        .iter()
        .map(|l| l.recorded)
        .collect();
    assert_eq!(recorded, [true, false]);
}

/// Stopping with no take under way keeps nothing.
#[test]
fn stopping_without_a_take_keeps_nothing() {
    let mut f = session("prompter-nostop", &[]);
    assert!(f.session.stop().unwrap().is_empty());
}

/// The script names each shot with its cue and its clip; one never
/// captured has none. Only a cued shot's clip is handed out.
#[test]
fn the_script_names_each_shot_and_its_clip() {
    let f = session("prompter-script", &[]);
    let script = f.session.script();
    assert_eq!(script.name, "tour.md");
    let texts: Vec<&str> = script.lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts, LINES);
    let captured = f.dir.join(format!("{}.mp4", Hash::of(b"welcome-a#0")));
    let clips: Vec<_> = script.shots.iter().map(|s| s.clip.clone()).collect();
    assert_eq!(clips, [None, Some(captured.clone()), None]);

    assert_eq!(
        f.session.clip(&Hash::of(b"welcome-a#0").to_string()),
        Some(captured)
    );
    let stray = Hash::of(b"not a cued shot");
    std::fs::write(f.dir.join(format!("{stray}.mp4")), b"clip").unwrap();
    assert_eq!(f.session.clip(&stray.to_string()), None);
    assert_eq!(f.session.clip("../secret"), None);
}

/// An edit reloads the script, but never under a take already playing
/// it: only between takes.
#[test]
fn a_script_is_reloaded_only_between_takes() {
    let mut f = session("prompter-replace", &[]);
    let mut edited = prompt(&f.dir);
    edited.shots[1].at = pos(0, 1);
    f.session.start(0);
    assert!(!f.session.replace(edited.clone()));
    assert_eq!(f.session.script().shots[1].at, pos(0, 3));
    f.session.stop().unwrap();
    assert!(f.session.replace(edited));
    assert_eq!(f.session.script().shots[1].at, pos(0, 1));
}

/// A line reworded since its take is stale, to be recorded again; the
/// reader is followed through its new words.
#[test]
fn a_reworded_line_is_stale_and_followed_as_it_now_reads() {
    let mut f = session("prompter-reworded", &["deployment is just"]);
    let takes = f.dir.join("takes");
    let pcm = teleprompt_voice::Pcm {
        sample_rate: 16_000,
        channels: 1,
        samples: vec![0; 16_000],
    };
    let mut store = teleprompt_voice::takes::Takes::load(&takes).unwrap();
    store.save("deploy", LINES[1], &pcm).unwrap();
    let mut edited = prompt(&f.dir);
    edited.lines[1] = "Deployment is just one command.".into();
    assert!(f.session.replace(edited));
    let script = f.session.script();
    assert_eq!(script.lines[1].text, "Deployment is just one command.");
    assert!(script.lines[1].stale && !script.lines[1].recorded);
    assert!(!script.lines[0].stale);
    f.session.start(1);
    assert_eq!(f.session.listen(&silence(1600), 16_000).at, pos(1, 3));
}

/// The take is heard again whole, and each line kept gets its part of
/// what was heard: where it is other words, the script offers the line as
/// said.
#[test]
fn a_line_kept_saying_other_words_offers_them() {
    const LINE_0: &str = "welcome to acme let me show you around";
    let mut heard = vec![""; 5];
    heard.extend(["welcome to"; 6]);
    heard.extend([LINE_0; 7]);
    heard.extend([""; 5]);
    heard.extend(["deployment is"; 7]);
    // Heard again, the kept line says less, and the next one begins.
    heard.extend(["welcome to acme let me show you deployment is"; 40]);
    let mut f = session("prompter-said", &heard);
    f.session.start(0);
    let take = audio(&[(0.3, false), (1.2, true), (0.8, false), (0.7, true)]);
    for chunk in take.chunks(1_600) {
        f.session.listen(chunk, 16_000);
    }
    assert_eq!(f.session.stop().unwrap(), ["welcome"]);

    let script = f.session.script();
    assert!(script.lines[0].recorded);
    assert_eq!(
        script.lines[0].said.as_deref(),
        Some("Welcome to Acme. Let me show you.")
    );
    assert_eq!(script.lines[1].said, None);
    let sidecar = std::fs::read_to_string(f.dir.join("takes/welcome.json")).unwrap();
    assert!(
        sidecar.contains("\"heard\": \"welcome to acme let me show you\""),
        "{sidecar}"
    );
}

/// Reads line 0 in full, as `stopping_a_take_keeps_each_line_read_in_full`
/// does, and returns the fixture with that take under way.
fn read_line_0(tag: &str) -> Fixture {
    const LINE_0: &str = "welcome to acme let me show you around";
    let mut heard = vec![""; 5];
    heard.extend(["welcome to"; 6]);
    heard.extend([LINE_0; 7]);
    heard.extend([""; 5]);
    let mut f = session(tag, &heard);
    f.session.start(0);
    for chunk in audio(&[(0.3, false), (1.2, true), (0.8, false)]).chunks(1_600) {
        f.session.listen(chunk, 16_000);
    }
    f
}

/// A take discarded keeps nothing, whatever was read.
#[test]
fn a_discarded_take_keeps_nothing() {
    let mut f = read_line_0("prompter-discard");
    f.session.discard();
    assert!(f.session.stop().unwrap().is_empty(), "the take is gone");
    let takes = teleprompt_voice::takes::Takes::load(&f.dir.join("takes")).unwrap();
    assert!(takes.is_empty());
}

/// The lines a take kept can be put back as they were, once.
#[test]
fn the_last_take_kept_can_be_undone() {
    let mut f = read_line_0("prompter-undo");
    assert_eq!(f.session.stop().unwrap(), ["welcome"]);
    assert!(f.session.script().lines[0].recorded);
    assert_eq!(f.session.undo().unwrap(), ["welcome"]);
    assert!(!f.session.script().lines[0].recorded, "no take before it");
    assert!(f.session.undo().unwrap().is_empty(), "once");
}

#[test]
fn word_starts_spread_a_line_over_its_characters() {
    use teleprompt_serve::prompter::word_starts;
    // 20 characters, words at 0, 6 and 13.
    assert_eq!(
        word_starts("Hello world, friend!", 2000),
        vec![0, 600, 1300]
    );
    assert_eq!(word_starts("", 1000), Vec::<u64>::new());
    assert_eq!(word_starts("One", 0), vec![0]);
}

#[test]
fn a_deaf_session_hears_nothing_and_stays_where_it_started() {
    let mut session = teleprompt_serve::prompter::Session::new(
        teleprompt_serve::prompter::Prompt {
            name: "s.md".into(),
            lines: vec!["One two.".into()],
            ids: vec!["one".into()],
            shots: Vec::new(),
            clips: teleprompt::project::CacheDir::at(std::env::temp_dir()),
            takes: std::env::temp_dir().join("teleprompt-deaf-takes"),
        },
        teleprompt_listen::Deaf,
    )
    .unwrap();
    session.start(0);
    let reached = session.listen(&[0.5; 1600], 16_000);
    assert_eq!((reached.at.line, reached.at.word), (0, 0));
}
