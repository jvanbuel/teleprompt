//! `teleprompt record` end to end: the binary in a terminal, asciinema
//! recording the shell typed into there, ffmpeg playing the recognizer's
//! fixture in real time as the microphone, and the model transcribing it.
//! A terminal, not a pipe: asciinema 3 records only what is typed at one.
//! Needs ffmpeg and a model (`TELEPROMPT_LISTEN_MODEL`); skipped without
//! them unless `TELEPROMPT_REQUIRE_LISTEN` is set.
#![cfg(all(unix, feature = "listen"))]

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

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

    let said = record_in_terminal(dir.path(), &model, &voice, reading, terminate);
    if terminate {
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

/// `teleprompt record` run in a terminal: `ls` typed between the two
/// readings, then the shell ended (or, with `terminate`, the recording
/// stopped with SIGTERM). What the terminal showed.
fn record_in_terminal(
    dir: &std::path::Path,
    model: &std::path::Path,
    voice: &std::path::Path,
    reading: Duration,
    terminate: bool,
) -> String {
    let pty = native_pty_system()
        .openpty(PtySize {
            cols: 80,
            rows: 24,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_teleprompt"));
    command.cwd(dir);
    command.args(["record", "scripts/session.md", "--model"]);
    command.arg(model);
    command.arg("--mic");
    command.arg(format!("-re -i {}", voice.display()));
    command.arg("--status");
    command.arg(dir.join("status.json"));
    if terminate {
        command.arg("--quiet");
    }
    command.args(["--", "sh"]);
    let mut record = pty.slave.spawn_command(command).unwrap();
    drop(pty.slave);
    // What the terminal shows, read as it comes so the program never
    // blocks writing to it.
    let shown = Arc::new(Mutex::new(Vec::new()));
    let mut reader = pty.master.try_clone_reader().unwrap();
    let into = Arc::clone(&shown);
    std::thread::spawn(move || {
        let mut buf = [0; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            into.lock().unwrap().extend_from_slice(&buf[..n]);
        }
    });
    let shown = || String::from_utf8_lossy(&shown.lock().unwrap()).into_owned();
    let mut keys = pty.master.take_writer().unwrap();
    // Typed in the pause between the two readings.
    std::thread::sleep(reading + Duration::from_millis(2000));
    for k in ["l", "s", "\r"] {
        keys.write_all(k.as_bytes()).unwrap();
        std::thread::sleep(Duration::from_millis(80));
    }
    std::thread::sleep(reading + Duration::from_millis(4000));
    if terminate {
        let pid = record.process_id().unwrap().to_string();
        assert!(Command::new("kill")
            .args(["-TERM", &pid])
            .status()
            .unwrap()
            .success());
    } else {
        keys.write_all(b"exit\r").unwrap();
    }
    // A recording whose shell never ends fails the test, not hangs it.
    let started = Instant::now();
    let status = loop {
        if let Some(status) = record.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > Duration::from_secs(120) {
            let _ = record.kill();
            panic!(
                "`teleprompt record` had not exited after 2 minutes:\n{}",
                shown()
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    // What it printed after the shell, once the terminal has drained.
    std::thread::sleep(Duration::from_millis(300));
    let said = shown();
    assert!(status.success(), "{said}");
    said
}
