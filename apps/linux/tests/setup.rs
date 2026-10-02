//! Setting teleprompt up from the app: what `teleprompt setup --uses` says
//! each use needs, and an install's progress as its events say.

use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};

use teleprompt_gtk::setup::{install, parse_step, parse_uses, use_for_adapter, uses, Step};

/// As `teleprompt --format json setup --uses` printed it, cut to three.
const USES: &str = include_str!("../../fixtures/setup-uses.json");

#[test]
fn each_use_says_what_it_still_needs() {
    let uses = parse_uses(USES).unwrap();
    let state: Vec<(&str, String)> = uses.iter().map(|u| (u.name.as_str(), u.state())).collect();
    assert_eq!(
        state,
        [
            ("render", "Installed".to_string()),
            ("terminal", "Needs vhs, ttyd".to_string()),
            (
                "conversations",
                "Needs teleprompt built with speech models".to_string()
            ),
        ]
    );
    let conversations = &uses[2];
    assert_eq!(conversations.missing().len(), 3);
    assert_eq!(conversations.download_mb, 388);
    // A tool that would ask for a password says so, for the app to warn.
    assert!(uses[1].tools.iter().any(|t| t.name == "ttyd" && t.password));
}

#[test]
fn an_install_event_is_a_step_and_anything_else_is_not() {
    let step = |s: &str| parse_step(s);
    assert_eq!(
        step(
            r#"{"event":"progress","stage":"install","state":"downloading","tool":"speech-model","mb":12,"of":310}"#
        ),
        Some(Step::Downloading {
            tool: "speech-model".into(),
            mb: 12,
            of: 310
        })
    );
    assert_eq!(
        step(r#"{"event":"progress","stage":"install","state":"done","tool":"vhs"}"#),
        Some(Step::Done { tool: "vhs".into() })
    );
    assert_eq!(
        step(r#"{"event":"progress","stage":"capture","state":"start","tool":"x"}"#),
        None
    );
    assert_eq!(step("  % Total    % Received"), None);
}

#[test]
fn an_adapter_is_set_up_by_its_use() {
    assert_eq!(use_for_adapter("vhs"), Some("terminal"));
    assert_eq!(use_for_adapter("playwright"), Some("browser"));
    assert_eq!(use_for_adapter("x11"), None);
}

/// A stand-in `teleprompt` that installs by saying it did, or fails as
/// the real one does.
fn stub(name: &str, script: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("tp-gtk-setup-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bin = dir.join("teleprompt");
    std::fs::write(&bin, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

#[test]
fn an_install_reports_each_step_as_it_goes() {
    let bin = stub(
        "ok",
        r#"echo "$@" > "$(dirname "$0")/args"
echo '{"event":"progress","stage":"install","state":"start","tool":"punctuation-model"}' >&2
echo '  % Total    % Received' >&2
echo '{"event":"progress","stage":"install","state":"downloading","tool":"punctuation-model","mb":8,"of":31}' >&2
echo '{"event":"progress","stage":"install","state":"done","tool":"punctuation-model"}' >&2
echo '{"ok": true}'"#,
    );
    let steps = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&steps);
    install(&bin, &["drafts".to_string()], move |s| {
        seen.lock().unwrap().push(s)
    })
    .unwrap();
    let steps = steps.lock().unwrap();
    assert_eq!(steps.len(), 3);
    assert_eq!(
        steps[2],
        Step::Done {
            tool: "punctuation-model".into()
        }
    );
    let args = std::fs::read_to_string(bin.with_file_name("args")).unwrap();
    assert_eq!(args.trim(), "--format json setup drafts --run");
}

#[test]
fn a_failed_install_says_why_as_teleprompt_does() {
    let bin = stub(
        "fail",
        r#"echo '{"ok": false, "errors": ["`sudo apt-get install -y ttyd` asks for your password, and nothing here can ask for it: run it in a terminal"]}'
exit 1"#,
    );
    let err = install(&bin, &["terminal".to_string()], |_| {}).unwrap_err();
    assert!(err.contains("asks for your password"), "{err}");
    let err = uses(&bin).unwrap_err();
    assert!(err.contains("asks for your password"), "{err}");
}
