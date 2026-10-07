//! A Rust plugin's side: a [`ScenePlugin`] answering
//! teleprompt over stdin and stdout, so one crate can be compiled into
//! teleprompt or built as an executable of its own:
//!
//! ```no_run
//! fn main() -> std::io::Result<()> {
//!     # let plugin = teleprompt_scene::ScenePlugin::new(
//!     #     teleprompt_scene::MockScene,
//!     #     teleprompt_scene::capture::mock::MockCapture::default(),
//!     # );
//!     teleprompt_scene::protocol::serve::scene(plugin)
//! }
//! ```

use std::io::{BufRead, Write};

use serde::de::DeserializeOwned;
use serde::Serialize;
use teleprompt_core::{BlockId, Hash, ShotId};

use super::{
    Body, Capture, Captured, Description, LineError, Need, Part, Retime, Retimed, Shots,
    Validation, VERSION,
};
use crate::capture::Session;
use crate::ScenePlugin;
use crate::{BlockSource, BodyOrigin, Measured, Shot, Validated};

/// Serves `plugin` on stdin and stdout until teleprompt closes stdin.
pub fn scene(plugin: ScenePlugin) -> std::io::Result<()> {
    scene_on(&plugin, std::io::stdin().lock(), std::io::stdout().lock())
}

/// One request: its method and parameters, and where to answer it.
struct Request<'a, W: Write> {
    id: serde_json::Value,
    method: String,
    params: serde_json::Value,
    out: &'a mut W,
}

impl<W: Write> Request<'_, W> {
    fn params<T: DeserializeOwned>(&self) -> Result<T, String> {
        serde_json::from_value(self.params.clone())
            .map_err(|e| format!("bad parameters for `{}`: {e}", self.method))
    }

    fn send(&mut self, message: serde_json::Value) -> std::io::Result<()> {
        writeln!(self.out, "{message}")?;
        self.out.flush()
    }

    fn answer(&mut self, result: Result<impl Serialize, String>) -> std::io::Result<()> {
        let id = self.id.clone();
        self.send(match result {
            Ok(r) => serde_json::json!({ "id": id, "result": r }),
            Err(e) => serde_json::json!({ "id": id, "error": e }),
        })
    }
}

/// Reads requests from `input` until it ends, passing each to `handle`.
fn serve<W: Write>(
    input: impl BufRead,
    mut out: W,
    mut handle: impl FnMut(&mut Request<'_, W>) -> std::io::Result<()>,
) -> std::io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let message: serde_json::Value = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(e) => {
                let error = format!("not a JSON request: {e}");
                writeln!(out, "{}", serde_json::json!({ "id": null, "error": error }))?;
                continue;
            }
        };
        let mut request = Request {
            id: message["id"].clone(),
            method: message["method"].as_str().unwrap_or_default().to_string(),
            params: message["params"].clone(),
            out: &mut out,
        };
        handle(&mut request)?;
    }
    Ok(())
}

fn unknown(method: &str) -> Result<(), String> {
    Err(super::unknown_method(method))
}

/// Serves `plugin` on `input` and `out`; for tests, and for a plugin that
/// talks over something other than its own stdin and stdout.
pub fn scene_on(plugin: &ScenePlugin, input: impl BufRead, out: impl Write) -> std::io::Result<()> {
    let scene = plugin.scene();
    serve(input, out, |r| match r.method.as_str() {
        "describe" => {
            let d = Description {
                protocol: VERSION,
                name: plugin.name().to_string(),
                needs: plugin.needs().iter().map(|t| Need::of(t)).collect(),
                continues: scene.continues(),
            };
            r.answer(Ok(d))
        }
        "validate" => {
            let answer = r.params::<Body>().map(|b| {
                // Located as an included file's, so line N is body line N-1.
                let src = BlockSource {
                    scene: b.scene,
                    body: b.body,
                    origin: BodyOrigin::Included {
                        path: String::new(),
                    },
                };
                let errors = scene.validate(&src).err().unwrap_or_default();
                Validation {
                    errors: errors
                        .into_iter()
                        .map(|d| LineError {
                            line: d.span.map(|s| s.line.saturating_sub(1)),
                            message: d.message,
                            help: d.help,
                        })
                        .collect(),
                }
            });
            r.answer(answer)
        }
        "shots" => {
            let answer = r.params::<Body>().and_then(|b| {
                let v = Validated {
                    scene: b.scene,
                    body: b.body,
                };
                let shots = scene.shots(&v, &BlockId::new("block")).map_err(|d| {
                    d.into_iter()
                        .map(|d| d.message)
                        .collect::<Vec<_>>()
                        .join("; ")
                })?;
                let shots = shots
                    .into_iter()
                    .map(|s| Part {
                        source: s.source,
                        length: s.length,
                    })
                    .collect();
                Ok(Shots { shots })
            });
            r.answer(answer)
        }
        "retime" => {
            let answer = r.params::<Retime>().map(|t| Retimed {
                source: scene.retime(&shot(t.source), t.target_ms),
            });
            r.answer(answer)
        }
        "capture" => capture(plugin, r),
        other => r.answer(unknown(other)),
    })
}

/// A shot of `source` alone, for `retime`, which is given a source.
fn shot(source: String) -> Shot {
    let hash = Hash::of(source.as_bytes());
    Shot {
        id: ShotId::new("block#0"),
        source,
        hash,
        index: 0,
        length: Measured::Unknown,
    }
}

fn capture<W: Write>(plugin: &ScenePlugin, r: &mut Request<'_, W>) -> std::io::Result<()> {
    let asked = match r.params::<Capture>() {
        Ok(c) => c,
        Err(e) => return r.answer(Err::<(), _>(e)),
    };
    let session = Session {
        plugin: plugin.name().to_string(),
        ..asked.session
    };
    let id = r.id.clone();
    let mut sent: std::io::Result<()> = Ok(());
    let clips = {
        let mut progress = |p: crate::capture::Progress| {
            if sent.is_ok() {
                sent = r.send(serde_json::json!({ "id": id, "progress": p }));
            }
        };
        plugin
            .capture()
            .capture(&session, &asked.frame, &asked.out_dir, &mut progress)
    };
    sent?;
    let clips = clips
        .map(|clips| Captured { clips })
        .map_err(|e| e.to_string());
    r.answer(clips)
}
