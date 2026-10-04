//! `teleprompt record` end to end: the binary, asciinema recording a shell
//! typed into through a pipe, ffmpeg playing the recognizer's fixture in
//! real time as the microphone, and the model transcribing it. Needs ffmpeg and a model
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
/// if it had exited. The app runs it quiet, leaving its terminal to the shell.
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
        .arg("--status")
        .arg(dir.join("status.json"))
        .args(terminate.then_some("--quiet"))
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
    if terminate {
        let said = format!("{}{stderr}", String::from_utf8_lossy(&out.stdout));
        assert!(
            !said.contains("recording ") && !said.contains("transcribing"),
            "{said}"
        );
    }

    let status: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("status.json")).unwrap()).unwrap();
    assert_eq!(status["state"], "done", "{status}");
    assert!(
        status["script"].as_str().unwrap().ends_with("session.md"),
        "{status}"
    );

    let md = std::fs::read_to_string(dir.join("scripts/session.md")).unwrap();
    let first = md.find("Welcome to acme").unwrap_or_else(|| panic!("{md}"));
    let block = md
        .find("scene=asciinema include=recordings/session.cast#1")
        .unwrap_or_else(|| panic!("{md}"));
    let second = md.rfind("Welcome to acme").unwrap();
    assert!(first < block && block < second, "{md}");
    // Recorded by asciinema, and cut before the closing `exit`.
    let cast = std::fs::read_to_string(dir.join("scripts/recordings/session.cast")).unwrap();
    assert!(
        cast.contains(r#""i","l""#) || cast.contains(r#""i","ls"#),
        "{cast}"
    );
    assert!(!cast.contains("exit"), "{cast}");
    let mut takes: Vec<String> = std::fs::read_dir(dir.join("takes"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        // `.previous` holds what an undo puts back, and no take.
        .filter(|name| !name.starts_with('.'))
        .collect();
    takes.sort();
    assert_eq!(
        takes.len(),
        4,
        "two takes, each a WAV and a sidecar: {takes:?}"
    );
}
