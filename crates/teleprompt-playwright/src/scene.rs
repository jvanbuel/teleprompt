//! A browser scene, driven by a Playwright script.
//!
//! The block *is* the script, run as written. A script states no timing,
//! so each shot's length is `Unknown`, `retime` keeps the contract's
//! `None`, and the shot takes its line's length (`docs/design.md#scene-plugins`).
//! A block may instead include one test of a test file ([`crate::spec`]).

use teleprompt_core::Hash;
use teleprompt_core::{BlockId, Diagnostic};
use teleprompt_plugin::scene::contract::{BlockSource, SceneCompiler, Shot, Validated};

/// The mark, spelled as a JavaScript comment so the script still runs
/// unchanged (`docs/design.md#marks`).
const MARK: &str = "// mark";

#[derive(Debug, Default, Clone, Copy)]
pub struct PlaywrightScene;

impl SceneCompiler for PlaywrightScene {
    fn kind(&self) -> &'static str {
        "playwright"
    }

    /// Accepts any body. This crate does not parse JavaScript, and a wrong
    /// script fails when it runs with Playwright's own, better, error.
    fn validate(&self, src: &BlockSource) -> Result<Validated, Vec<Diagnostic>> {
        Ok(Validated::from(src))
    }

    fn shots(&self, v: &Validated, block_id: &BlockId) -> Result<Vec<Shot>, Vec<Diagnostic>> {
        if crate::spec::is_spec(&v.body) {
            return Err(vec![Diagnostic::error(
                "this is a Playwright test file: name the test to run, \
                 e.g. `include=\"checkout.spec.ts#pays by card\"`",
            )]);
        }
        let mut out = Vec::new();
        for chunk in v.body.split('\n').fold(vec![Vec::new()], |mut acc, line| {
            if line.trim() == MARK {
                acc.push(Vec::new());
            } else {
                acc.last_mut().expect("seeded with one").push(line);
            }
            acc
        }) {
            let source = chunk.join("\n");
            // A chunk of only comments and blank lines is no shot.
            if !has_code(&source) {
                continue;
            }
            let index = out.len();
            let hash = Hash::of(source.as_bytes());
            out.push(Shot::numbered(block_id, index, source, hash));
        }
        Ok(out)
    }

    /// In a test file, the test with that title; in a script, `#2` is the
    /// part after its first `// mark` and `#2-3` a range.
    fn select(&self, body: &str, fragment: &str) -> Result<String, String> {
        if crate::spec::is_spec(body) {
            crate::spec::test_body(body, fragment)
        } else {
            teleprompt_plugin::scene::select_marked(body, MARK, fragment)
        }
    }
}

/// Whether a chunk holds anything that would run.
fn has_code(source: &str) -> bool {
    source
        .lines()
        .map(str::trim)
        .any(|l| !l.is_empty() && !l.starts_with("//"))
}
