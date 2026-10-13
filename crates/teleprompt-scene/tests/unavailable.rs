//! A capture backend that runs exactly what it needs says it cannot run
//! when, and only when, a program it needs is not on PATH, naming the tools
//! `teleprompt setup` installs.

use std::path::Path;

use teleprompt_scene::capture::{CaptureBackend, CaptureError, Clip, Frame, Progress, Session};
use teleprompt_scene::core::tool::{Found, Tool, Unavailable};

static GONE: Tool = Tool {
    name: "gone",
    what: "",
    license: "",
    home: "",
    guide: None,
    found: Found::Program("teleprompt-test-no-such-program"),
    install: &[],
    download_mb: None,
};

static SHELL: Tool = Tool {
    name: "sh",
    found: Found::Program("sh"),
    ..GONE
};

/// A package, which no PATH check can find, is not reported missing.
static PACKAGE: Tool = Tool {
    name: "package",
    found: Found::Package("teleprompt-test-no-such-package"),
    ..GONE
};

struct Needs(&'static [&'static Tool]);

impl CaptureBackend for Needs {
    fn needs(&self) -> &'static [&'static Tool] {
        self.0
    }

    fn capture(
        &self,
        _session: &Session,
        _frame: &Frame,
        _out_dir: &Path,
        _on_progress: &mut dyn FnMut(Progress),
    ) -> Result<Vec<Clip>, CaptureError> {
        Ok(Vec::new())
    }
}

#[test]
fn a_backend_is_missing_the_programs_it_needs_and_nothing_else() {
    static NEEDS: &[&Tool] = &[&SHELL, &GONE, &PACKAGE];
    let why = Needs(NEEDS).unavailable().expect("one program is missing");

    let Unavailable::Missing(tools) = &why else {
        panic!("not a missing tool: {why}");
    };
    assert_eq!(tools.iter().map(|t| t.name).collect::<Vec<_>>(), ["gone"]);
    assert_eq!(
        why.to_string(),
        "teleprompt-test-no-such-program is not on PATH"
    );
}

#[test]
fn a_backend_with_everything_it_needs_is_available() {
    static NEEDS: &[&Tool] = &[&SHELL, &PACKAGE];
    let why = Needs(NEEDS).unavailable();
    assert!(why.is_none(), "{why:?}");
}
