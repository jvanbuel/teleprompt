//! A Rust plugin's side: an [`ScenePlugin`] or [`VoicePlugin`] answering
//! teleprompt over stdin and stdout, so one crate can be compiled into
//! teleprompt or built as an executable of its own:
//!
//! ```no_run
//! fn main() -> std::io::Result<()> {
//!     # let plugin = teleprompt_plugin::ScenePlugin::new(
//!     #     teleprompt_plugin::scene::MockScene,
//!     #     teleprompt_plugin::capture::mock::MockCapture::default(),
//!     # );
//!     teleprompt_plugin::protocol::serve::scene(plugin)
//! }
//! ```

use std::io::{BufRead, Write};
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::Serialize;
use teleprompt_core::{BlockId, Hash, ShotId};

use super::{
    Body, Capture, Captured, Configure, Configured, Description, LineError, Probed, Retime,
    Retimed, SceneTraits, Shots, Spoken, Synthesize, Traits, Unavailable, Validation, VoiceTraits,
    Voices, WireClip, WireProgress, WireShot, VERSION,
};
use crate::capture::{Frame, Session, SessionShot};
use crate::scene::{BlockSource, BodyOrigin, Measured, Shot, Validated};
use crate::voice::{wav, SynthRequest, VoiceBackend, VoicePlugin};
use crate::ScenePlugin;

/// Serves `plugin` on stdin and stdout until teleprompt closes stdin.
pub fn scene(plugin: ScenePlugin) -> std::io::Result<()> {
    scene_on(&plugin, std::io::stdin().lock(), std::io::stdout().lock())
}

/// Serves `voice` on stdin and stdout until teleprompt closes stdin.
pub fn voice(voice: VoicePlugin) -> std::io::Result<()> {
    voice_on(&voice, std::io::stdin().lock(), std::io::stdout().lock())
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
    Err(format!("unknown method `{method}`"))
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
                needs: plugin.needs().iter().map(|t| t.to_wire()).collect(),
                traits: Traits::Scene(SceneTraits {
                    continues: scene.continues(),
                    retimes: true,
                }),
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
                    .iter()
                    .map(|s| timed(s.source.clone(), scene.estimate(s)))
                    .collect();
                Ok(Shots { shots })
            });
            r.answer(answer)
        }
        "estimate" => {
            let answer = r
                .params::<WireShot>()
                .map(|w| timed(String::new(), scene.estimate(&shot(w.source))));
            r.answer(answer)
        }
        "retime" => {
            let answer = r.params::<Retime>().map(|t| Retimed {
                source: scene.retime(&shot(t.source), t.target_ms),
            });
            r.answer(answer)
        }
        "unavailable" => r.answer(Ok(Unavailable {
            reason: plugin.capture().unavailable(),
        })),
        "capture" => capture(plugin, r),
        other => r.answer(unknown(other)),
    })
}

/// A shot of `source` alone, for the methods that are given a source.
fn shot(source: String) -> Shot {
    let hash = Hash::of(source.as_bytes());
    Shot {
        id: ShotId::new("block#0"),
        source,
        hash,
        index: 0,
    }
}

fn timed(source: String, m: Measured) -> WireShot {
    let (ms, exact) = match m {
        Measured::Exact(ms) => (Some(ms), true),
        Measured::Estimated(ms) => (Some(ms), false),
        Measured::Unknown => (None, false),
    };
    WireShot { source, ms, exact }
}

fn capture<W: Write>(adapter: &ScenePlugin, r: &mut Request<'_, W>) -> std::io::Result<()> {
    let asked = match r.params::<Capture>() {
        Ok(c) => c,
        Err(e) => return r.answer(Err::<(), _>(e)),
    };
    let shots: Result<Vec<SessionShot>, String> = asked
        .session
        .shots
        .into_iter()
        .map(|s| {
            let key: Hash = serde_json::from_value(serde_json::Value::String(s.key))
                .map_err(|e| format!("shot `{}`: {e}", s.id))?;
            Ok(SessionShot {
                id: ShotId::new(s.id),
                key,
                source: s.source,
                duration_ms: s.duration_ms,
                wanted: s.wanted,
            })
        })
        .collect();
    let shots = match shots {
        Ok(s) => s,
        Err(e) => return r.answer(Err::<(), _>(e)),
    };
    let session = Session {
        scene: asked.session.scene,
        plugin: adapter.name().to_string(),
        name: asked.session.name,
        settings: asked.session.settings,
        root: asked.session.root,
        shots,
    };
    let frame = Frame {
        width: asked.frame.width,
        height: asked.frame.height,
        fps: asked.frame.fps,
    };
    let id = r.id.clone();
    let mut sent: std::io::Result<()> = Ok(());
    let clips = {
        let mut progress = |p: crate::capture::Progress| {
            let event = WireProgress {
                shot: p.shot.to_string(),
                done: p.done,
                of: p.of,
            };
            if sent.is_ok() {
                sent = r.send(serde_json::json!({ "id": id, "progress": event }));
            }
        };
        adapter
            .capture()
            .capture(&session, &frame, &asked.out_dir, &mut progress)
    };
    sent?;
    let clips = clips
        .map(|clips| Captured {
            clips: clips
                .into_iter()
                .map(|c| WireClip {
                    key: c.key.to_string(),
                    path: c.path,
                })
                .collect(),
        })
        .map_err(|e| e.to_string());
    r.answer(clips)
}

/// Serves `voice` on `input` and `out`, as [`scene_on`] does a scene plugin.
pub fn voice_on(voice: &VoicePlugin, input: impl BufRead, out: impl Write) -> std::io::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut backend: Option<Arc<dyn VoiceBackend>> = None;
    serve(input, out, |r| match r.method.as_str() {
        "describe" => {
            // What it can do, as its defaults show: settings come after.
            let probe = (voice.build)(None).ok();
            let caps = probe.as_ref().map(|b| b.capabilities());
            let d = Description {
                protocol: VERSION,
                name: voice.id.to_string(),
                needs: voice.needs.iter().map(|t| t.to_wire()).collect(),
                traits: Traits::Voice(VoiceTraits {
                    word_timings: caps.as_ref().is_some_and(|c| c.word_timings),
                    speed_control: caps.as_ref().is_some_and(|c| c.speed_control),
                    address: probe.as_ref().and_then(|b| b.address()),
                    lists_voices: true,
                    probes: probe.as_ref().is_some_and(|b| b.address().is_some()),
                }),
            };
            r.answer(Ok(d))
        }
        "configure" => {
            let answer = r.params::<Configure>().and_then(|c| {
                let settings: Option<serde_yaml::Value> = c
                    .settings
                    .map(|s| serde_yaml::to_value(s).map_err(|e| e.to_string()))
                    .transpose()?;
                let built = (voice.build)(settings.as_ref())?;
                let version = built.capabilities().version;
                backend = Some(built);
                Ok(Configured { version })
            });
            r.answer(answer)
        }
        "synthesize" => {
            let answer = configured(&backend).and_then(|b| {
                let s = r.params::<Synthesize>()?;
                let req = SynthRequest {
                    text: s.text,
                    locale: s.locale,
                    voice: s.voice,
                    speed: s.speed,
                    instruct: s.instruct,
                };
                let spoken = runtime
                    .block_on(b.synthesize(&req))
                    .map_err(|e| e.to_string())?;
                std::fs::write(&s.out, wav::encode(&spoken.pcm))
                    .map_err(|e| format!("cannot write {}: {e}", s.out.display()))?;
                Ok(Spoken {
                    word_timings: spoken.word_timings,
                })
            });
            r.answer(answer)
        }
        "voices" => {
            let answer = configured(&backend).and_then(|b| match runtime.block_on(b.voices()) {
                Some(listed) => listed
                    .map(|voices| Voices { voices })
                    .map_err(|e| e.to_string()),
                None => Err(format!("`{}` does not list its voices", voice.id)),
            });
            r.answer(answer)
        }
        "probe" => {
            let answer = configured(&backend).and_then(|b| {
                runtime
                    .block_on(b.probe())
                    .map(|line| Probed { line })
                    .map_err(|e| e.to_string())
            });
            r.answer(answer)
        }
        other => r.answer(unknown(other)),
    })
}

fn configured(backend: &Option<Arc<dyn VoiceBackend>>) -> Result<Arc<dyn VoiceBackend>, String> {
    backend
        .clone()
        .ok_or_else(|| "asked before `configure`".to_string())
}
