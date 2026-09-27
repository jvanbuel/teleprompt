//! `teleprompt record` end to end: the binary, a shell typed into through
//! a pipe, ffmpeg playing the recognizer's fixture in real time as the
//! microphone, and the model transcribing it. Needs ffmpeg and a model
//! (`TELEPROMPT_LISTEN_MODEL`); skipped without them unless
//! `TELEPROMPT_REQUIRE_LISTEN` is set.
#![cfg(all(unix, feature = "listen"))]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use teleprompt_voice::{wav, Pcm};

#[test]
fn a_recorded_session_becomes_a_script_spoken_in_the_recording() {
    session("record-listen", false);
}

/// The app's Stop: SIGTERM ends the shell, and the session is imported as
/// if it had exited.
#[test]
fn a_recording_stopped_with_sigterm_is_still_imported() {
    session("record-sigterm", true);
}

fn session(tag: &str, terminate: bool) {
    let required = std::env::var_os("TELEPROMPT_REQUIRE_LISTEN").is_some();
    let model = std::env::var_os("TELEPROMPT_LISTEN_MODEL").map(PathBuf::from);
    let ffmpeg = Command::new("ffmpeg").arg("-version").output().is_ok();
    assert!(
        !required || (model.is_some() && ffmpeg),
        "needs a model and ffmpeg"
    );
    let Some(model) = model.filter(|_| ffmpeg) else {
        return;
    };

    let dir = teleprompt_testkit::test_dir(tag);
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../teleprompt-listen-sherpa/tests/fixtures/two-lines.wav");
    let once = wav::decode(&std::fs::read(fixture).unwrap()).unwrap();
    let reading = Duration::from_millis(once.duration_ms());
    let gap = vec![0i16; once.sample_rate as usize * 5];
    let voice = dir.join("voice.wav");
    std::fs::write(
        &voice,
        wav::encode(&Pcm {
            samples: [once.samples.clone(), gap, once.samples.clone()].concat(),
            ..once.clone()
        }),
    )
    .unwrap();

    let mut record = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .current_dir(dir.path())
        .args(["record", "scripts/session.md", "--model"])
        .arg(&model)
        .arg("--mic")
        .arg(format!("-re -i {}", voice.display()))
        .args(["--", "sh"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut keys = record.stdin.take().unwrap();
    // Typed in the pause between the two readings.
    std::thread::sleep(reading + Duration::from_millis(2000));
    for k in ["l", "s", "\r"] {
        keys.write_all(k.as_bytes()).unwrap();
        std::thread::sleep(Duration::from_millis(80));
    }
    std::thread::sleep(reading + Duration::from_millis(4000));
    if terminate {
        let pid = record.id().to_string();
        assert!(Command::new("kill")
            .args(["-TERM", &pid])
            .status()
            .unwrap()
            .success());
    } else {
        keys.write_all(b"exit\r").unwrap();
    }
    let out = record.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");

    let md = std::fs::read_to_string(dir.join("scripts/session.md")).unwrap();
    let first = md.find("Welcome to acme").unwrap_or_else(|| panic!("{md}"));
    let tape = md.find("Type \"ls\"").unwrap_or_else(|| panic!("{md}"));
    let second = md.rfind("Welcome to acme").unwrap();
    assert!(first < tape && tape < second, "{md}");
    assert!(md.contains("Set TypingSpeed 80ms"), "{md}");
    assert!(!md.contains("exit"), "{md}");
    let takes = std::fs::read_dir(dir.join("takes")).unwrap().count();
    assert_eq!(takes, 4, "two takes, each a WAV and a sidecar");
}
