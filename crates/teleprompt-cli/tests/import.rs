//! `teleprompt import`: a recorded session and its voice become a script
//! and the takes for its lines. Words come from a file here, so these run
//! without a speech model; the Speech workflow runs one.

use std::path::{Path, PathBuf};

use teleprompt_cli::cmd::import::{run_import, Import, Words};
use teleprompt_cli::cmd::{check::run_check, plan::run_plan};
use teleprompt_cli::project::Project;
use teleprompt_voice::takes::Takes;
use teleprompt_voice::{wav, Pcm};

const RATE: u32 = 24_000;

struct Session {
    _dir: teleprompt_testkit::TestDir,
    root: PathBuf,
    cast: PathBuf,
    voice: PathBuf,
    words: PathBuf,
}

/// "Let's see what is here." from 1 s, `ls -la` typed during it, and
/// "And now the file." from 6 s, with `cat notes.txt` typed in the pause
/// between; 10 s of tone standing in for the voice.
fn session() -> Session {
    let dir = teleprompt_testkit::test_dir("import");
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    let root = dir.to_path_buf();

    let mut cast = String::from("{\"version\": 2, \"width\": 80, \"height\": 24}\n");
    let mut t = 1.9;
    for c in "ls -la\r".chars() {
        cast.push_str(&format!(
            "[{t:.3}, \"i\", {}]\n",
            serde_json::to_string(&c.to_string()).unwrap()
        ));
        t += 0.06;
    }
    cast.push_str("[2.5, \"o\", \"a.txt\\r\\n\"]\n");
    t = 4.0;
    for c in "cat notes.txt\r".chars() {
        cast.push_str(&format!(
            "[{t:.3}, \"i\", {}]\n",
            serde_json::to_string(&c.to_string()).unwrap()
        ));
        t += 0.06;
    }
    let cast_path = root.join("session.cast");
    std::fs::write(&cast_path, cast).unwrap();

    let words: Vec<serde_json::Value> = [(1000, "LET'S SEE WHAT IS HERE"), (6000, "AND NOW THE FILE")]
        .iter()
        .flat_map(|(at, text)| {
            text.split_whitespace().enumerate().map(move |(i, w)| {
                serde_json::json!({"text": w, "start_ms": at + i * 400, "end_ms": at + i * 400 + 350})
            })
        })
        .collect();
    let words_path = root.join("words.json");
    std::fs::write(&words_path, serde_json::to_vec(&words).unwrap()).unwrap();

    let samples = (0..RATE * 10)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect();
    let voice = root.join("voice.wav");
    std::fs::write(
        &voice,
        wav::encode(&Pcm {
            sample_rate: RATE,
            channels: 1,
            samples,
        }),
    )
    .unwrap();

    Session {
        _dir: dir,
        cast: cast_path,
        voice,
        words: words_path,
        root,
    }
}

fn import(
    s: &Session,
    script: &Path,
    force: bool,
) -> Result<teleprompt_cli::cmd::import::ImportReport, String> {
    run_import(&Import {
        cast: &s.cast,
        voice: &s.voice,
        script,
        words: Words::File(&s.words),
        offset_ms: 0,
        force,
    })
}

#[test]
fn a_session_becomes_a_script_that_checks() {
    let s = session();
    let script = s.root.join("scripts/tour.md");
    let report = import(&s, &script, false).unwrap();
    assert_eq!((report.lines, report.tapes), (2, 2));

    let md = std::fs::read_to_string(&script).unwrap();
    assert!(
        md.contains("\n# Tour\n"),
        "titled from the file name:\n{md}"
    );
    assert!(md.contains("Let's see what is here.\n"), "{md}");
    assert!(
        md.contains("```teleprompt scene=terminal policy=concurrent cue=\"what is\"\n"),
        "{md}"
    );
    assert!(md.contains("Type \"cat notes.txt\"\n"), "{md}");

    let project = Project::for_script(&script).unwrap();
    let warnings = run_check(&project, &script, "en").unwrap();
    assert!(
        warnings.iter().all(|w| !w.contains("error")),
        "{warnings:?}"
    );
}

/// Each line's take is its stretch of the recording: from just before its
/// first word to just after its last, never into the next line's.
#[test]
fn each_line_gets_its_take() {
    let s = session();
    let script = s.root.join("scripts/tour.md");
    let report = import(&s, &script, false).unwrap();

    let project = Project::for_script(&script).unwrap();
    let plan = run_plan(&project, &script, "en").unwrap();
    let lines: Vec<_> = plan
        .narration
        .iter()
        .map(|n| (n.line_id.clone(), n.text.clone()))
        .collect();
    assert_eq!(
        report.takes,
        lines.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>()
    );

    let takes = Takes::load(&project.takes_dir()).unwrap();
    // "Let's see what is here." runs 1000–2950 ms: 150 ms before, 250 after.
    let first = takes
        .current(&lines[0].0, &lines[0].1)
        .expect("the first line is recorded");
    assert_eq!(first.duration_ms, 2350);
    let second = takes.current(&lines[1].0, &lines[1].1).unwrap();
    assert_eq!(second.duration_ms, 1950);
    let pcm = wav::decode(&takes.read(&lines[0].0).unwrap()).unwrap();
    assert_eq!((pcm.sample_rate, pcm.channels), (RATE, 1));

    let recorded = plan
        .timeline
        .entries
        .iter()
        .filter_map(|e| e.narration.as_ref())
        .filter(|n| n.recorded);
    assert_eq!(recorded.count(), 2);
}

/// Importing twice would overwrite the script and its takes: refused
/// without `--force`.
#[test]
fn an_existing_script_is_not_overwritten() {
    let s = session();
    let script = s.root.join("scripts/tour.md");
    import(&s, &script, false).unwrap();
    let e = import(&s, &script, false).unwrap_err();
    assert!(e.contains("already exists") && e.contains("--force"), "{e}");
    import(&s, &script, true).unwrap();
}

/// Without a speech model or a words file there is nothing to hear the
/// voice with, and without keystrokes nothing to script.
#[test]
fn a_cast_without_keystrokes_is_refused() {
    let s = session();
    std::fs::write(&s.cast, "{\"version\": 2}\n[0.5, \"o\", \"$ \"]\n").unwrap();
    let e = import(&s, &s.root.join("scripts/tour.md"), false).unwrap_err();
    assert!(e.contains("no keystrokes"), "{e}");
}
