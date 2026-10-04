//! A motion scene: a composition from an existing Remotion project.
//!
//! A shot names one of the project's compositions and its props, as
//! `npx remotion render <id> --props=…` takes them.
//!
//! ```text
//! Title {"title": "Hello"}
//! ```
//!
//! A composition takes as long as it is told to, so `estimate` is
//! [`Measured::Unknown`] and the scheduler gives each shot its sentence.
//! `retime` always succeeds, because the length is a render parameter.

use std::path::{Path, PathBuf};

use teleprompt_core::config::SceneConfig;
use teleprompt_core::{BlockId, Diagnostic, Hash};
use teleprompt_plugin::scene::contract::{
    split_at_mark, validate_parts, BlockSource, Measured, SceneCompiler, Shot, Validated,
};

/// The mark, and `#` comments generally — the shell's spelling, since a
/// shot reads like the arguments to `remotion render`. A shot after a mark
/// has no sentence to take its length from, and `check` refuses it.
pub const MARK: &str = "# mark";

/// How a re-timed shot states its length, so the length is in its hash.
const LENGTH_PREFIX: &str = "# teleprompt: ";
const LENGTH_SUFFIX: &str = "ms";

/// A shot, read: which composition, and its props as a JSON object.
#[derive(Debug, Clone, PartialEq)]
pub struct Invocation {
    pub composition: String,
    pub props: serde_json::Value,
}

/// Read one shot's source. `Ok(None)` for a shot of only comments.
pub fn parse(source: &str) -> Result<Option<Invocation>, String> {
    let text: String = source
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let (composition, props) = text
        .split_once(char::is_whitespace)
        .map_or((text, ""), |(id, rest)| (id, rest.trim()));
    // Remotion's own rule for a composition id.
    if !composition.chars().all(|c| c.is_alphanumeric() || c == '-') {
        return Err(format!(
            "`{composition}` is not a composition id: Remotion allows letters, digits and `-`"
        ));
    }
    let props = if props.is_empty() {
        serde_json::json!({})
    } else {
        match serde_json::from_str::<serde_json::Value>(props) {
            Ok(v) if v.is_object() => v,
            Ok(_) => return Err("props must be a JSON object".into()),
            Err(e) => {
                return Err(format!(
                    "props for `{composition}` are not JSON: {e} (one composition per \
                     shot: give the next its own paragraph and block)"
                ))
            }
        }
    };
    Ok(Some(Invocation {
        composition: composition.to_string(),
        props,
    }))
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RemotionScene;

impl SceneCompiler for RemotionScene {
    fn kind(&self) -> &'static str {
        "remotion"
    }

    /// Every shot must name a composition and give props as JSON. Whether
    /// the composition exists is the project's business, and Remotion says
    /// so when it is asked to render it.
    fn validate(&self, src: &BlockSource) -> Result<Validated, Vec<Diagnostic>> {
        validate_parts(src, MARK, |part| parse(part).map(|_| ()))
    }

    fn shots(&self, v: &Validated, block_id: &BlockId) -> Result<Vec<Shot>, Vec<Diagnostic>> {
        let mut out = Vec::new();
        for (_, source) in split_at_mark(&v.body, MARK) {
            if !matches!(parse(&source), Ok(Some(_))) {
                continue;
            }
            let index = out.len();
            let hash = Hash::of(source.as_bytes());
            out.push(Shot::numbered(block_id, index, source, hash));
        }
        Ok(out)
    }

    fn estimate(&self, _shot: &Shot) -> Measured {
        Measured::Unknown
    }

    /// The same shot, stating the length it will be rendered at. The
    /// length has to be in the source because the source is what the
    /// capture key is built from: a composition that animates across its
    /// duration is a different picture at four seconds than at six.
    fn retime(&self, shot: &Shot, target_ms: u64) -> Option<String> {
        let body = match shot.source.split_once('\n') {
            Some((first, rest))
                if first.starts_with(LENGTH_PREFIX) && first.ends_with(LENGTH_SUFFIX) =>
            {
                rest
            }
            _ => &shot.source,
        };
        Some(format!("{LENGTH_PREFIX}{target_ms}{LENGTH_SUFFIX}\n{body}"))
    }

    /// What Remotion's bundler reads, by Remotion's own layout: the
    /// directory holding the entry point, `public/` for `staticFile`, the
    /// package manifests that pin its dependencies, and its config. Not
    /// the whole project, which defaults to `.` and would be the whole
    /// repository.
    fn inputs(&self, scene: &SceneConfig) -> Vec<PathBuf> {
        let setting = |key: &str, default: &str| {
            scene
                .settings
                .get(key)
                .and_then(|v| v.as_str())
                .unwrap_or(default)
                .to_string()
        };
        let project = scene.path("project", ".");
        let entry = project.join(setting("entry", "src/index.ts"));
        let mut out: Vec<PathBuf> = entry.parent().map(Path::to_path_buf).into_iter().collect();
        out.extend(
            [
                "public",
                "package.json",
                "package-lock.json",
                "remotion.config.ts",
            ]
            .iter()
            .map(|p| project.join(p)),
        );
        out
    }

    /// A composition draws the same frames whatever was on screen before
    /// it, so each shot is named by itself and editing one re-renders one.
    fn continues(&self) -> bool {
        false
    }
}
