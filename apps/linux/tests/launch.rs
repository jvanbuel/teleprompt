mod common;

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use teleprompt_gtk::launch::{
    exit_reasons, parse_listening, LaunchEvent, LaunchRequest, ServerProcess,
};

fn request(binary: PathBuf) -> LaunchRequest {
    LaunchRequest {
        binary,
        script: "/p/scripts/tour.md".into(),
        model: "/models/zipformer".into(),
        locale: "en".into(),
    }
}

#[test]
fn the_command_asks_for_json_and_a_free_port() {
    assert_eq!(
        request("/bin/teleprompt".into()).args(),
        [
            "--format",
            "json",
            "prompt",
            "/p/scripts/tour.md",
            "--locale",
            "en",
            "--port",
            "0",
            "--model",
            "/models/zipformer",
        ]
    );
}

#[test]
fn the_listening_example_names_the_origin() {
    let line = common::example("listening.json");
    assert_eq!(
        parse_listening(line.trim()).as_deref(),
        Some("http://127.0.0.1:7879")
    );
    assert_eq!(
        parse_listening(r#"{"event":"listening","url":"http://127.0.0.1:1","api":"/api/v2"}"#),
        None
    );
    assert_eq!(parse_listening("prompting at http://127.0.0.1:1/"), None);
}

#[test]
fn an_exit_says_why() {
    let report = b"{\n  \"ok\": false,\n  \"errors\": [\n    \"no model\"\n  ]\n}\n";
    assert_eq!(exit_reasons(report, b"", Some(1)), ["no model"]);
    let stderr: String = (1..=8).map(|i| format!("line {i}\n")).collect();
    assert_eq!(
        exit_reasons(b"", stderr.as_bytes(), Some(1)),
        (4..=8).map(|i| format!("line {i}")).collect::<Vec<_>>()
    );
    assert_eq!(
        exit_reasons(b"", b"", Some(9)),
        ["teleprompt exited with status 9"]
    );
}

/// A stand-in for `teleprompt`: a shell script.
fn fake(body: &str) -> (PathBuf, tempdir::Dir) {
    let dir = tempdir::Dir::new();
    let path = dir.0.join("teleprompt");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    (path, dir)
}

fn launch(body: &str) -> (ServerProcess, mpsc::Receiver<LaunchEvent>, tempdir::Dir) {
    let (binary, dir) = fake(body);
    let (tx, rx) = mpsc::channel();
    // Another test forking while this one wrote its script holds it open
    // for writing until that child execs: running it is busy, briefly.
    for _ in 0..50 {
        let tx = tx.clone();
        match ServerProcess::start(&request(binary.clone()), move |e| {
            let _ = tx.send(e);
        }) {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                std::thread::sleep(Duration::from_millis(10));
            }
            started => return (started.unwrap(), rx, dir),
        }
    }
    panic!("{} stayed busy", binary.display())
}

#[test]
fn a_server_that_listens_says_where_and_stops_when_asked() {
    let (server, events, _dir) = launch(
        "echo 'prompting at http://127.0.0.1:4242/' >&2\n\
         echo '{\"event\":\"listening\",\"url\":\"http://127.0.0.1:4242\",\"api\":\"/api/v1\"}'\n\
         exec sleep 30",
    );
    let wait = Duration::from_secs(5);
    assert_eq!(
        events.recv_timeout(wait).unwrap(),
        LaunchEvent::Listening("http://127.0.0.1:4242".into())
    );
    server.stop();
    assert!(matches!(
        events.recv_timeout(wait).unwrap(),
        LaunchEvent::Ended(_)
    ));
}

#[test]
fn a_server_that_fails_says_why() {
    let (_server, events, _dir) = launch(
        "printf '{\\n  \"ok\": false,\\n  \"errors\": [\\n    \"no speech model\"\\n  ]\\n}\\n'\nexit 1",
    );
    assert_eq!(
        events.recv_timeout(Duration::from_secs(5)).unwrap(),
        LaunchEvent::Ended(vec!["no speech model".into()])
    );
}

/// Dropping the server stops it: an app that goes does not leave it behind.
#[test]
fn a_dropped_server_is_stopped() {
    let (server, events, _dir) = launch(
        "echo '{\"event\":\"listening\",\"url\":\"http://127.0.0.1:4242\",\"api\":\"/api/v1\"}'\nexec sleep 30",
    );
    events.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(server);
    assert!(matches!(
        events.recv_timeout(Duration::from_secs(5)).unwrap(),
        LaunchEvent::Ended(_)
    ));
}

mod tempdir {
    use std::path::PathBuf;

    /// A directory removed when dropped.
    pub struct Dir(pub PathBuf);

    impl Dir {
        pub fn new() -> Self {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static N: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "teleprompt-gtk-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::SeqCst)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

/// An app that dies without stopping its server, killed or crashed, takes
/// the server with it: nothing is left holding the port or the takes.
#[test]
fn a_server_goes_with_the_app_that_launched_it() {
    let (binary, _dir) = fake(
        "echo '{\"event\":\"listening\",\"url\":\"http://127.0.0.1:4242\",\"api\":\"/api/v1\"}'\nexec sleep 30",
    );
    let (tx, events) = mpsc::channel();
    // The launching thread stands in for the app: once the server listens
    // it goes, and the server is never dropped.
    let wait = Duration::from_secs(5);
    std::thread::spawn(move || {
        let (listening_tx, listening) = mpsc::channel();
        let server = ServerProcess::start(&request(binary), move |e| {
            if matches!(e, LaunchEvent::Listening(_)) {
                let _ = listening_tx.send(());
            }
            let _ = tx.send(e);
        })
        .unwrap();
        listening.recv_timeout(wait).unwrap();
        std::mem::forget(server);
    })
    .join()
    .unwrap();
    assert!(matches!(
        events.recv_timeout(wait).unwrap(),
        LaunchEvent::Listening(_)
    ));
    assert!(
        matches!(events.recv_timeout(wait), Ok(LaunchEvent::Ended(_))),
        "the server outlived the app"
    );
}
