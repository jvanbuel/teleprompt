//! What `serve` holds while it runs, and the session over its socket.

use super::*;

/// Clears a flag when dropped.
pub(super) struct Flag<'a>(pub(super) &'a AtomicBool);

impl Drop for Flag<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Marks the session closed when dropped.
pub(super) struct OpenSession(pub(super) Arc<Server>);

impl Drop for OpenSession {
    fn drop(&mut self) {
        self.0.open.store(false, Ordering::SeqCst);
    }
}

pub(super) struct Server {
    /// What this build has, for setting it up from the page.
    pub(super) registry: Registry,
    /// The open script's session; none until a script is opened.
    pub(super) session: Mutex<Option<Session<Hearing>>>,
    /// What can be done to the open script's file.
    pub(super) edits: RwLock<Option<Arc<Edits>>>,
    /// How to open a script from the page; none where only the one given
    /// is served.
    pub(super) opener: Option<Opener>,
    /// Whether a session socket is open.
    pub(super) open: AtomicBool,
    /// Whether a capture, build or install is running.
    pub(super) making: AtomicBool,
}

impl Server {
    pub(super) fn new(
        registry: Registry,
        opened: Option<Opened>,
        opener: Option<Opener>,
    ) -> Arc<Self> {
        let (session, edits) = match opened {
            Some(o) => (Some(o.session), Some(Arc::new(o.edits))),
            None => (None, None),
        };
        Arc::new(Self {
            registry,
            session: Mutex::new(session),
            edits: RwLock::new(edits),
            opener,
            open: AtomicBool::new(false),
            making: AtomicBool::new(false),
        })
    }

    pub(super) fn session(&self) -> MutexGuard<'_, Option<Session<Hearing>>> {
        self.session.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn edits(&self) -> Option<Arc<Edits>> {
        self.edits
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn opened_name(&self) -> Option<String> {
        self.session().as_ref().map(|s| s.script().name)
    }

    /// The script as the page draws it, reloaded first if its file has
    /// changed; compiled outside the lock, which the session needs. None
    /// while no script is open.
    pub(super) fn script(&self) -> Option<serde_json::Value> {
        let edits = self.edits();
        let edited = edits.as_ref().and_then(|e| (e.reload)());
        let mut session = self.session();
        let session = session.as_mut()?;
        if let Some(prompt) = edited {
            session.replace(prompt);
        }
        let mut script = script(session.script());
        if let Some(edits) = &edits {
            voiced(&mut script, edits);
        }
        Some(script)
    }

    pub(super) async fn run_session(&self, mut socket: WebSocket) {
        let mut rate = LISTEN_RATE;
        let mut at = None;
        while let Some(Ok(message)) = socket.recv().await {
            let answer = match message {
                Message::Text(text) => {
                    block_in_place(|| self.command(text.as_str(), &mut rate, &mut at))
                }
                Message::Binary(audio) => {
                    let reached = block_in_place(|| {
                        self.session()
                            .as_mut()
                            .map(|s| s.listen(&samples(&audio), rate))
                    });
                    let Some(reached) = reached else {
                        return;
                    };
                    let news = at != Some(reached.at) || !reached.play.is_empty();
                    at = Some(reached.at);
                    news.then(|| reached_json(&reached))
                }
                Message::Close(_) => return,
                _ => None,
            };
            if let Some(answer) = answer {
                if socket
                    .send(Message::Text(answer.to_string().into()))
                    .await
                    .is_err()
                {
                    return;
                }
            }
        }
    }

    /// A JSON message from the client, and the answer to it.
    pub(super) fn command(
        &self,
        text: &str,
        rate: &mut u32,
        at: &mut Option<Position>,
    ) -> Option<serde_json::Value> {
        let message: serde_json::Value = match serde_json::from_str(text) {
            Ok(message) => message,
            Err(e) => return Some(error(format!("not JSON: {e}"))),
        };
        let edits = self.edits();
        let mut session = self.session();
        let Some(session) = session.as_mut() else {
            return Some(error("no script is open".into()));
        };
        match message["type"].as_str() {
            Some("start") if !edits.as_ref().is_none_or(|e| e.listens) => Some(error(
                "this prompter reads the script with its voice; it does not listen".into(),
            )),
            Some("start") => {
                let from = message["from"].as_u64().unwrap_or(0) as usize;
                *rate = message["rate"]
                    .as_u64()
                    .and_then(|r| u32::try_from(r).ok())
                    .filter(|&r| r > 0)
                    .unwrap_or(LISTEN_RATE);
                let reached = session.start(from);
                *at = Some(reached.at);
                Some(reached_json(&reached))
            }
            Some("stop") => Some(match session.stop() {
                Ok(saved) => serde_json::json!({ "type": "stopped", "saved": saved }),
                Err(e) => error(e.to_string()),
            }),
            Some("discard") => {
                session.discard();
                Some(serde_json::json!({ "type": "discarded" }))
            }
            Some("undo") => Some(match session.undo() {
                Ok(lines) => serde_json::json!({ "type": "undone", "lines": lines }),
                Err(e) => error(e.to_string()),
            }),
            Some("keep_said") => {
                let line = message["line"].as_str().unwrap_or_default();
                Some(match &edits {
                    Some(edits) => match (edits.keep_said)(line) {
                        Ok(()) => serde_json::json!({ "type": "kept_said", "line": line }),
                        Err(e) => error(e),
                    },
                    None => error("this prompter has no script file to reword".into()),
                })
            }
            Some("undo_edit") => Some(match &edits {
                Some(edits) => match (edits.undo)() {
                    Ok(()) => serde_json::json!({ "type": "edit_undone" }),
                    Err(e) => error(e),
                },
                None => error("this prompter has no script file to edit".into()),
            }),
            Some("reword" | "instruct" | "cue" | "hold" | "move" | "stretch") => {
                let (edit, edited) = match edit_of(&message) {
                    Ok(edit) => edit,
                    Err(e) => return Some(error(e)),
                };
                Some(match &edits {
                    Some(edits) => match (edits.edit)(&edit) {
                        Ok(()) => edited,
                        Err(e) => error(e),
                    },
                    None => error("this prompter has no script file to edit".into()),
                })
            }
            _ => Some(error(format!("not a message this server knows: {text}"))),
        }
    }
}

/// The edit a message asks for, and the answer once it is made: naming
/// the line it changed, or the block it moved.
pub(super) fn edit_of(message: &serde_json::Value) -> Result<(Edit, serde_json::Value), String> {
    let text = |key: &str| message[key].as_str().map(str::to_string);
    let line = || text("line").ok_or("which line? `line` names it");
    let block = || text("block").ok_or("which block? `block` names it");
    let word = || message["word"].as_u64().map(|w| w as usize);
    let edit = match message["type"].as_str().unwrap_or_default() {
        "reword" => Edit::Reword {
            line: line()?.into(),
            text: text("text").ok_or("a reword needs its text")?,
        },
        "instruct" => Edit::Instruct {
            line: line()?.into(),
            text: text("text").filter(|t| !t.trim().is_empty()),
        },
        "cue" => Edit::Cue {
            block: block()?.into(),
            word: word().ok_or("a cue needs its `word`")?,
        },
        "hold" => Edit::Hold {
            block: block()?.into(),
        },
        "move" => Edit::Move {
            block: block()?.into(),
            after: text("after")
                .ok_or("a move needs the line it goes `after`")?
                .into(),
            word: word(),
        },
        _ => Edit::Stretch {
            block: block()?.into(),
            by: message["by"]
                .as_f64()
                .filter(|by| *by > 0.0 && by.is_finite())
                .ok_or("a stretch needs `by`, more than 0")?,
        },
    };
    let edited = match &edit {
        Edit::Reword { line, .. } | Edit::Instruct { line, .. } => {
            serde_json::json!({ "type": "edited", "line": line })
        }
        Edit::Cue { block, .. }
        | Edit::Hold { block }
        | Edit::Move { block, .. }
        | Edit::Stretch { block, .. } => serde_json::json!({ "type": "edited", "block": block }),
    };
    Ok((edit, edited))
}

pub(super) fn reached_json(r: &Reached) -> serde_json::Value {
    serde_json::json!({ "type": "reached", "line": r.at.line, "word": r.at.word, "play": r.play })
}

pub(super) fn error(message: String) -> serde_json::Value {
    serde_json::json!({ "type": "error", "message": message })
}

pub(super) fn script(s: ScriptView) -> serde_json::Value {
    let lines: Vec<_> = s
        .lines
        .iter()
        .map(|l| {
            let diff = l.said.as_deref().map_or_else(Vec::new, |said| {
                teleprompt_core::said::diff(&l.text, said)
            });
            serde_json::json!({ "id": l.id, "text": l.text, "recorded": l.recorded, "stale": l.stale, "said": l.said, "said_diff": diff })
        })
        .collect();
    let shots: Vec<_> = s
        .shots
        .iter()
        .map(|shot| {
            serde_json::json!({
                "shot": shot.shot,
                "at": { "line": shot.at.line, "word": shot.at.word },
                "clip": shot.clip.as_ref().map(|_| format!("/api/v1/clips/{}.mp4", shot.capture_key)),
            })
        })
        .collect();
    serde_json::json!({ "name": s.name, "lines": lines, "shots": shots })
}

/// `script` with who reads it, how long it runs, and each line's audio.
pub(super) fn voiced(script: &mut serde_json::Value, edits: &Edits) {
    let Some(voice) = edits.voice.as_ref().and_then(Voicing::describe) else {
        return;
    };
    script["voice"] = serde_json::json!({ "name": voice.name, "listens": edits.listens });
    script["length_ms"] = voice.length_ms.into();
    script["timeline"] = voice.timeline;
    script["error"] = voice.error.into();
    let Some(lines) = script["lines"].as_array_mut() else {
        return;
    };
    for line in lines {
        let id = line["id"].as_str().unwrap_or_default().to_string();
        if let Some((_, audio)) = voice.lines.iter().find(|(l, _)| *l == id) {
            line["audio"] = audio["audio"].clone();
            line["instruct"] = audio["instruct"].clone();
            line["speaker"] = audio["speaker"].clone();
        }
    }
}

/// Little-endian f32 samples, as the page sends them.
pub(super) fn samples(body: &[u8]) -> Vec<f32> {
    body.as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}
