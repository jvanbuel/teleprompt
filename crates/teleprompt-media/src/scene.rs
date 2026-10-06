//! A media scene: one directive per shot.
//!
//! ```text
//! image src=arch.png fit=contain
//! clip  src=office.mp4 from=0:12 to=0:19
//! title text="Part Two" subtitle="Configuration"
//! ```
//!
//! Images and titles are stills and take their sentence's length. A clip
//! with `from` and `to` is exact and never re-timed; without them it plays
//! for its sentence. `src` is relative to the scene's `dir`.

use std::path::PathBuf;

use teleprompt_plugin::core::attrs::parse_attrs;
use teleprompt_plugin::core::config::SceneConfig;
use teleprompt_plugin::core::{BlockId, Diagnostic, Hash, SourceSpan};
use teleprompt_plugin::scene::contract::{
    is_content, BlockSource, Measured, SceneCompiler, Shot, Validated,
};

/// The mark, and `#` comments generally.
pub const MARK: &str = "# mark";

/// One shot, read.
#[derive(Debug, Clone, PartialEq)]
pub enum Directive {
    Image {
        src: String,
        cover: bool,
    },
    Clip {
        src: String,
        from_ms: u64,
        to_ms: Option<u64>,
        cover: bool,
    },
    Title {
        text: String,
        subtitle: Option<String>,
    },
}

/// A time in a clip: `12`, `12.5s`, `250ms`, `0:12`, `1:02.5`.
pub fn parse_time(v: &str) -> Result<u64, String> {
    let bad = || format!("`{v}` is not a time: write `0:12`, `12.5s` or `250ms`");
    if v.contains(':') {
        let mut seconds = 0.0;
        for part in v.split(':') {
            let n: f64 = part.parse().map_err(|_| bad())?;
            seconds = seconds * 60.0 + n;
        }
        return Ok(teleprompt_plugin::core::time::ms_from_seconds(seconds));
    }
    if let Some(ms) = v.strip_suffix("ms") {
        return ms.parse().map_err(|_| bad());
    }
    let n: f64 = v
        .strip_suffix('s')
        .unwrap_or(v)
        .parse()
        .map_err(|_| bad())?;
    if n < 0.0 {
        return Err(bad());
    }
    Ok(teleprompt_plugin::core::time::ms_from_seconds(n))
}

/// Read one directive line. Errors are messages with an optional help.
pub fn parse_line(line: &str) -> Result<Directive, Diagnostic> {
    let line = line.trim();
    let (verb, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let span = SourceSpan {
        line: 0,
        column: 1,
        len: 0,
    };
    let allowed: &[&str] = match verb {
        "image" => &["src", "fit"],
        "clip" => &["src", "from", "to", "fit"],
        "title" => &["text", "subtitle"],
        other => {
            return Err(
                Diagnostic::error(format!("`{other}` is not a media directive"))
                    .with_help("a media shot is `image src=…`, `clip src=…` or `title text=\"…\"`"),
            )
        }
    };
    let (attrs, mut diags) = parse_attrs(rest, allowed, span);
    if let Some(d) = diags.drain(..).next() {
        return Err(d);
    }
    let need = |key: &str| {
        attrs
            .get(key)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .ok_or_else(|| Diagnostic::error(format!("`{verb}` needs `{key}=`")))
    };
    let cover = match attrs.get("fit") {
        None | Some("contain") => false,
        Some("cover") => true,
        Some(other) => {
            return Err(Diagnostic::error(format!(
                "`fit={other}` is not `contain` or `cover`"
            )))
        }
    };
    let time = |key: &str| {
        attrs
            .get(key)
            .map(parse_time)
            .transpose()
            .map_err(Diagnostic::error)
    };
    Ok(match verb {
        "image" => Directive::Image {
            src: need("src")?,
            cover,
        },
        "clip" => {
            let from_ms = time("from")?.unwrap_or(0);
            let to_ms = time("to")?;
            if to_ms.is_some_and(|to| to <= from_ms) {
                return Err(Diagnostic::error("`to` must come after `from`"));
            }
            Directive::Clip {
                src: need("src")?,
                from_ms,
                to_ms,
                cover,
            }
        }
        _ => Directive::Title {
            text: need("text")?,
            subtitle: attrs.get("subtitle").map(str::to_string),
        },
    })
}

/// The directive a shot's source holds, if it holds one.
pub fn directive(source: &str) -> Option<Directive> {
    source
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .and_then(|l| parse_line(l).ok())
}

/// The block split at its marks, each chunk with the index of its first line.
fn chunks(body: &str) -> Vec<(usize, Vec<(usize, &str)>)> {
    let mut out = vec![(0, Vec::new())];
    for (i, line) in body.lines().enumerate() {
        if line.trim() == MARK {
            out.push((i + 1, Vec::new()));
        } else if is_content(line) {
            out.last_mut().expect("seeded").1.push((i, line));
        }
    }
    out
}

#[derive(Debug, Default, Clone, Copy)]
pub struct MediaScene;

pub fn length(source: &str) -> Measured {
    match directive(source) {
        Some(Directive::Clip {
            from_ms,
            to_ms: Some(to),
            ..
        }) => Measured::Exact(to - from_ms),
        _ => Measured::Unknown,
    }
}

impl SceneCompiler for MediaScene {
    fn kind(&self) -> &'static str {
        "media"
    }

    fn validate(&self, src: &BlockSource) -> Result<Validated, Vec<Diagnostic>> {
        let mut diags = Vec::new();
        for (_, lines) in chunks(&src.body) {
            for (n, (i, line)) in lines.iter().enumerate() {
                let d = if n > 0 {
                    Some(
                        Diagnostic::error("one directive per shot")
                            .with_help("give it a paragraph and a block of its own"),
                    )
                } else {
                    parse_line(line).err()
                };
                if let Some(d) = d {
                    diags.push(src.origin.locate(d, *i, line.trim().len()));
                }
            }
        }
        if diags.is_empty() {
            Ok(Validated::from(src))
        } else {
            Err(diags)
        }
    }

    fn shots(&self, v: &Validated, block_id: &BlockId) -> Result<Vec<Shot>, Vec<Diagnostic>> {
        let mut out = Vec::new();
        for (_, lines) in chunks(&v.body) {
            let Some((_, line)) = lines.first() else {
                continue;
            };
            let source = line.trim().to_string();
            let index = out.len();
            let (hash, length) = (Hash::of(source.as_bytes()), length(&source));
            out.push(Shot::numbered(block_id, index, source, hash).lasting(length));
        }
        Ok(out)
    }

    /// The one file a shot shows, by content: replacing it re-renders
    /// the shots that show it and nothing else. A title shows none.
    fn shot_inputs(&self, scene: &SceneConfig, source: &str) -> Vec<PathBuf> {
        let dir = scene.path("dir", "media");
        match directive(source) {
            Some(Directive::Image { src, .. } | Directive::Clip { src, .. }) => {
                vec![dir.join(src)]
            }
            _ => Vec::new(),
        }
    }

    fn continues(&self) -> bool {
        false
    }
}
