//! The real `teleprompt prompt`, launched and driven as the app does it,
//! hearing a recorded reading of `apps/fixtures/tour`. Opt-in:
//! it needs a binary built with `--features listen` and a speech model.
//!
//!     TELEPROMPT_BIN=…/teleprompt TELEPROMPT_MODEL=…/zipformer cargo test --test end_to_end

mod common;

use std::sync::mpsc;
use std::time::Duration;

use teleprompt_gtk::api::{ClientMessage, Position, ServerMessage};
use teleprompt_gtk::launch::{LaunchEvent, LaunchRequest, ServerProcess};
use teleprompt_gtk::session::{Incoming, Outgoing, SessionClient};

#[test]
fn a_reading_is_followed_and_kept() {
    let (Ok(binary), Ok(model)) = (
        std::env::var("TELEPROMPT_BIN"),
        std::env::var("TELEPROMPT_MODEL"),
    ) else {
        // CI sets this, so a job that stops providing them cannot go
        // quietly green.
        assert!(
            std::env::var_os("TELEPROMPT_REQUIRE_E2E").is_none(),
            "TELEPROMPT_REQUIRE_E2E is set, but TELEPROMPT_BIN or TELEPROMPT_MODEL is not"
        );
        eprintln!("skipped: set TELEPROMPT_BIN and TELEPROMPT_MODEL");
        return;
    };
    // A copy, so the takes it records land nowhere that lasts.
    let project = std::env::temp_dir().join(format!("teleprompt-gtk-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&project);
    copy_dir(&common::repository().join("apps/fixtures/tour"), &project);

    let (events_tx, events) = mpsc::channel();
    let _server = ServerProcess::start(
        &LaunchRequest {
            binary: binary.into(),
            script: project.join("scripts/tour.md"),
            model: model.into(),
            locale: "en".into(),
        },
        move |e| {
            let _ = events_tx.send(e);
        },
    )
    .unwrap();
    let origin = match events.recv_timeout(Duration::from_secs(60)).unwrap() {
        LaunchEvent::Listening(origin) => origin,
        LaunchEvent::Ended(why) => panic!("the server did not listen: {why:?}"),
    };

    let client = SessionClient::new(origin);
    let lines = client.script().unwrap().lines;
    assert_eq!(lines.len(), 2);
    let (heard_tx, heard) = mpsc::channel();
    let socket = client
        .open(move |incoming| {
            let _ = heard_tx.send(incoming);
        })
        .unwrap();

    let (samples, rate) = read_wav(
        &common::repository().join("crates/teleprompt-listen-sherpa/tests/fixtures/two-lines.wav"),
    );
    socket
        .send(Outgoing::Command(ClientMessage::Start { from: 0, rate }))
        .unwrap();
    for chunk in samples.chunks(rate as usize / 10) {
        socket.send(Outgoing::Audio(chunk.to_vec())).unwrap();
    }
    // A second of silence, as a reader pauses before keeping a take.
    socket
        .send(Outgoing::Audio(vec![0.0; rate as usize]))
        .unwrap();
    socket.send(Outgoing::Command(ClientMessage::Stop)).unwrap();

    let mut furthest = Position::default();
    loop {
        match heard.recv_timeout(Duration::from_secs(60)).unwrap() {
            Incoming::Message(ServerMessage::Reached { at, .. }) => {
                assert!(
                    at >= furthest,
                    "the reader only moves on: {at:?} after {furthest:?}"
                );
                furthest = at;
            }
            Incoming::Message(ServerMessage::Stopped { saved }) => {
                assert_eq!(furthest.line, 2, "followed to the end");
                let ids: Vec<String> = lines.iter().map(|l| l.id.clone()).collect();
                assert_eq!(saved, ids, "both lines kept");
                let recorded: Vec<bool> = client
                    .script()
                    .unwrap()
                    .lines
                    .iter()
                    .map(|l| l.recorded)
                    .collect();
                assert_eq!(recorded, [true, true]);
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    socket.send(Outgoing::Close).unwrap();
    let _ = std::fs::remove_dir_all(&project);
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// 16-bit PCM mono WAV, as floats, and its rate.
fn read_wav(path: &std::path::Path) -> (Vec<f32>, u32) {
    let data = std::fs::read(path).unwrap();
    let u32_at = |at: usize| u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
    let u16_at = |at: usize| u16::from_le_bytes(data[at..at + 2].try_into().unwrap());
    let (mut at, mut rate) = (12, 0);
    while at + 8 <= data.len() {
        let size = u32_at(at + 4) as usize;
        match &data[at..at + 4] {
            b"fmt " => {
                assert_eq!(
                    (u16_at(at + 8), u16_at(at + 10), u16_at(at + 22)),
                    (1, 1, 16)
                );
                rate = u32_at(at + 12);
            }
            b"data" => {
                let samples = data[at + 8..at + 8 + size]
                    .chunks_exact(2)
                    .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
                    .collect();
                return (samples, rate);
            }
            _ => {}
        }
        at += 8 + size + size % 2;
    }
    panic!("no audio in {}", path.display());
}
