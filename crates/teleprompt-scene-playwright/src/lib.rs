//! A browser scene, driven by a Playwright script.
//!
//! The block *is* the script. There is no dialect here that resembles
//! Playwright — no `Click`, no `Goto`, nothing to learn and nothing to keep
//! in step with an API somebody else owns. What an author writes is what
//! Playwright runs, and everything Playwright can do is available without
//! this crate knowing about it.
//!
//! That choice decides the rest of the adapter. The tape adapter reads a
//! language whose every command states a duration, so it can say how long a
//! shot lasts and re-write it to last longer. A script states no such
//! thing: `await page.click(…)` takes as long as the page takes. So
//! `estimate` is [`Measured::Unknown`] and `retime` is `None` — the honest
//! answers, and the ones the contract already documents. The scheduler
//! gives the shot the length of the sentence spoken over it, the script
//! does its work inside that window, and the renderer holds the last frame
//! if the script finishes first.

use teleprompt_core::Diagnostic;
use teleprompt_core::Hash;
use teleprompt_scene::contract::{BlockSource, Measured, SceneCompiler, Shot, Validated};

/// The mark, spelled as a JavaScript comment.
///
/// A comment for the same reason the tape's mark is one: the block has to
/// stay something a person can paste into a Playwright test unchanged. A
/// mark that broke the script would make the demo a dialect again.
const MARK: &str = "// mark";

#[derive(Debug, Default, Clone, Copy)]
pub struct PlaywrightScene;

impl SceneCompiler for PlaywrightScene {
    fn kind(&self) -> &'static str {
        "playwright"
    }

    /// Nothing to validate but emptiness.
    ///
    /// It would be easy to reach further — reject `page.close()`, warn on a
    /// missing `await` — and every rule of that kind is this crate
    /// pretending to understand a language it does not parse. A script that
    /// is wrong fails when it runs, with Playwright's own error, which is a
    /// better message than any this crate could invent.
    fn validate(&self, src: &BlockSource) -> Result<Validated, Vec<Diagnostic>> {
        Ok(Validated {
            scene: src.scene.clone(),
            body: src.body.clone(),
        })
    }

    fn shots(&self, v: &Validated, block_id: &str) -> Result<Vec<Shot>, Vec<Diagnostic>> {
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
            // A chunk of only comments and blank lines does nothing, and a
            // shot that does nothing is a picture nobody asked for.
            if !has_code(&source) {
                continue;
            }
            let index = out.len();
            out.push(Shot {
                id: format!("{block_id}#{index}"),
                hash: Hash::of(source.as_bytes()),
                source,
                index,
            });
        }
        Ok(out)
    }

    /// How long a script takes is a question only running it can answer.
    fn estimate(&self, _shot: &Shot) -> Measured {
        Measured::Unknown
    }
}

/// Whether a chunk holds anything that would run.
fn has_code(source: &str) -> bool {
    source
        .lines()
        .map(str::trim)
        .any(|l| !l.is_empty() && !l.starts_with("//"))
}
