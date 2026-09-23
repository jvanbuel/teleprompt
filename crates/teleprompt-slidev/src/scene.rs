//! A slides scene: a slide of an existing Slidev deck, at a click step.
//!
//! A shot names a slide the way Slidev's own URLs do — `3` is slide three
//! as it opens, `3?clicks=2` is slide three after two clicks — so a slide's
//! `v-click`s can be stepped through a paragraph at a time, each block
//! showing the reveal its sentence is about.
//!
//! A still is the same picture at any length. So `estimate` is
//! [`Measured::Unknown`] — the scheduler gives the shot its sentence — and
//! there is no `retime`: the length is kept out of the key on purpose, and
//! rewording a sentence reuses the slide it was spoken over.

use std::path::PathBuf;

use teleprompt_core::config::SceneConfig;
use teleprompt_core::{Diagnostic, Hash};
use teleprompt_scene::contract::{BlockSource, Measured, SceneCompiler, Shot, Validated};

/// The mark, and `#` comments generally. A shot after a mark has no
/// sentence to take its length from, and `check` refuses it.
pub const MARK: &str = "# mark";

/// A shot, read: a 1-based slide number and how many clicks into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub slide: u32,
    pub clicks: u32,
}

/// Read one shot's source. `Ok(None)` for a shot of only comments.
pub fn parse(source: &str) -> Result<Option<Step>, String> {
    let lines: Vec<&str> = source
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    let line = match lines.as_slice() {
        [] => return Ok(None),
        [line] => *line,
        _ => {
            return Err(format!(
                "one slide per block: give `{}` and `{}` a paragraph each",
                lines[0], lines[1]
            ))
        }
    };
    let (slide, clicks) = match line.split_once('?') {
        None => (line, "0"),
        Some((slide, query)) => match query.strip_prefix("clicks=") {
            Some(clicks) => (slide, clicks),
            None => return Err(format!("`?{query}` is not `?clicks=N`")),
        },
    };
    let slide: u32 = slide
        .parse()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| format!("`{line}` is not a slide: write `3`, or `3?clicks=2`"))?;
    let clicks: u32 = clicks
        .parse()
        .map_err(|_| format!("`{clicks}` is not a number of clicks"))?;
    Ok(Some(Step { slide, clicks }))
}

/// The block split at its marks, each chunk with the index of its first line.
fn chunks(body: &str) -> Vec<(usize, String)> {
    let mut out = vec![(0, String::new())];
    for (i, line) in body.lines().enumerate() {
        if line.trim() == MARK {
            out.push((i + 1, String::new()));
        } else {
            let chunk = &mut out.last_mut().expect("seeded").1;
            chunk.push_str(line);
            chunk.push('\n');
        }
    }
    out
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SlidevScene;

impl SceneCompiler for SlidevScene {
    fn kind(&self) -> &'static str {
        "slidev"
    }

    /// Every shot must name one slide. Whether the deck has it is the
    /// deck's business, and the capture says so.
    fn validate(&self, src: &BlockSource) -> Result<Validated, Vec<Diagnostic>> {
        let diags: Vec<Diagnostic> = chunks(&src.body)
            .into_iter()
            .filter_map(|(first, chunk)| {
                let why = parse(&chunk).err()?;
                let at = chunk
                    .lines()
                    .position(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
                    .unwrap_or(0);
                Some(src.origin.locate(Diagnostic::error(why), first + at, 0))
            })
            .collect();
        if diags.is_empty() {
            Ok(Validated {
                scene: src.scene.clone(),
                body: src.body.clone(),
            })
        } else {
            Err(diags)
        }
    }

    fn shots(&self, v: &Validated, block_id: &str) -> Result<Vec<Shot>, Vec<Diagnostic>> {
        let mut out = Vec::new();
        for (_, source) in chunks(&v.body) {
            let Ok(Some(step)) = parse(&source) else {
                continue;
            };
            // Hashed in canonical form, so a comment or a `?clicks=0`
            // does not re-export a slide that did not change.
            let canonical = format!("{}?clicks={}", step.slide, step.clicks);
            let index = out.len();
            out.push(Shot {
                id: format!("{block_id}#{index}"),
                hash: Hash::of(canonical.as_bytes()),
                source,
                index,
            });
        }
        Ok(out)
    }

    fn estimate(&self, _shot: &Shot) -> Measured {
        Measured::Unknown
    }

    /// The deck and what Slidev reads beside it, by Slidev's own layout.
    fn inputs(&self, scene: &SceneConfig) -> Vec<PathBuf> {
        let deck = PathBuf::from(
            scene
                .settings
                .get("deck")
                .and_then(|v| v.as_str())
                .unwrap_or("slides.md"),
        );
        let dir = deck.parent().map(PathBuf::from).unwrap_or_default();
        let mut out = vec![deck];
        out.extend(
            [
                "components",
                "layouts",
                "public",
                "styles",
                "setup",
                "pages",
                "snippets",
                "package.json",
                "package-lock.json",
                "vite.config.ts",
                "uno.config.ts",
            ]
            .iter()
            .map(|p| dir.join(p)),
        );
        out
    }

    /// A slide at a click step is the same picture whatever came before.
    fn continues(&self) -> bool {
        false
    }
}
