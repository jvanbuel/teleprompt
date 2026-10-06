//! `teleprompt serve` without a script: the page's welcome lists the
//! project's scripts and opens one, and the page sets teleprompt up, as
//! `teleprompt setup` does.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::{Child, Command, Stdio};

struct Served {
    child: Child,
    addr: SocketAddr,
    dir: teleprompt_testkit::TestDir,
}

impl Drop for Served {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A scaffolded project, served with no script open.
fn home(tag: &str) -> Served {
    let dir = teleprompt_testkit::test_dir(tag);
    teleprompt::new::scaffold(&dir).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .current_dir(&dir)
        .args(["--format", "json", "serve", "--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut first = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut first)
        .unwrap();
    let event: serde_json::Value =
        serde_json::from_str(&first).unwrap_or_else(|e| panic!("{e}: {first:?}"));
    let addr = event["url"]
        .as_str()
        .unwrap()
        .trim_start_matches("http://")
        .parse()
        .unwrap();
    Served { child, addr, dir }
}

/// A request, and its status and body.
fn request(addr: SocketAddr, method: &str, path: &str) -> (u16, String) {
    let mut s = TcpStream::connect(addr).unwrap();
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    s.read_to_string(&mut response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    (head[9..12].parse().unwrap(), body.to_string())
}

fn json(body: &str) -> serde_json::Value {
    serde_json::from_str(body).unwrap_or_else(|e| panic!("{e}: {body}"))
}

/// With nothing open there is no script to show, and the welcome lists
/// the project's; opening one makes it the script the page shows.
#[test]
fn the_welcome_lists_the_projects_scripts_and_opens_one() {
    let served = home("serve-home");
    assert_eq!(request(served.addr, "GET", "/api/v1/script").0, 404);

    let (status, body) = request(served.addr, "GET", "/api/v1/home");
    assert_eq!(status, 200, "{body}");
    let welcome = json(&body);
    assert!(welcome["opened"].is_null(), "{welcome}");
    let scripts = welcome["scripts"].as_array().unwrap();
    assert_eq!(scripts.len(), 1, "{welcome}");
    assert_eq!(scripts[0]["name"], "demo.md");
    assert_eq!(welcome["can_hear"], cfg!(feature = "listen"), "{welcome}");

    let path = scripts[0]["path"].as_str().unwrap();
    let (status, body) = request(
        served.addr,
        "POST",
        &format!("/api/v1/open?script={}&voice=1", encode(path)),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(json(&body)["name"], "demo.md");

    let (status, body) = request(served.addr, "GET", "/api/v1/script");
    assert_eq!(status, 200, "{body}");
    let script = json(&body);
    assert!(!script["lines"].as_array().unwrap().is_empty(), "{script}");
    assert_eq!(script["voice"]["listens"], false, "{script}");
    assert_eq!(
        json(&request(served.addr, "GET", "/api/v1/home").1)["opened"],
        "demo.md"
    );
}

/// A script that does not compile is not opened, and says why; one read
/// by ear in a build that cannot hear says what to set up.
#[test]
fn a_script_that_cannot_be_opened_says_why() {
    let served = home("serve-home-refused");
    let broken = served.dir.join("scripts/broken.md");
    std::fs::write(
        &broken,
        "---\nteleprompt: 1\n---\n\nA line.\n\n```teleprompt scene=nowhere\nx\n```\n",
    )
    .unwrap();
    let (status, body) = request(
        served.addr,
        "POST",
        &format!(
            "/api/v1/open?script={}&voice=1",
            encode(broken.to_str().unwrap())
        ),
    );
    assert_eq!(status, 422, "{body}");
    assert!(
        !json(&body)["errors"].as_array().unwrap().is_empty(),
        "{body}"
    );

    if !cfg!(feature = "listen") {
        let demo = served.dir.join("scripts/demo.md");
        let (status, body) = request(
            served.addr,
            "POST",
            &format!("/api/v1/open?script={}", encode(demo.to_str().unwrap())),
        );
        assert_eq!(status, 409, "{body}");
        assert_eq!(json(&body)["needs"], "prompt", "{body}");
    }
    assert_eq!(request(served.addr, "GET", "/api/v1/script").0, 404);
}

/// What teleprompt can be set up to do is the report `setup --uses`
/// gives; installing names what to set up, and only what `setup` knows.
#[test]
fn the_page_sets_teleprompt_up_as_setup_does() {
    let served = home("serve-home-setup");
    let (status, body) = request(served.addr, "GET", "/api/v1/setup");
    assert_eq!(status, 200, "{body}");
    let uses = json(&body);
    let names: Vec<&str> = uses["uses"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|u| u["name"].as_str())
        .collect();
    assert!(
        names.contains(&"render") && names.contains(&"prompt"),
        "{names:?}"
    );

    assert_eq!(request(served.addr, "POST", "/api/v1/setup").0, 400);
    let (status, body) = request(
        served.addr,
        "POST",
        "/api/v1/setup?uses=nothing-of-the-sort",
    );
    assert_eq!(status, 400, "{body}");
}

/// A path as a query parameter.
fn encode(path: &str) -> String {
    path.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}
