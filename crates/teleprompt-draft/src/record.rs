//! `teleprompt record <script> --with <plugin>`: the plugin's own tool
//! records the author at work (asciinema, `vhs record`, `playwright
//! codegen`), and ffmpeg records the microphone beside it. When the tool
//! finishes, the two are drafted into `<script>` (`crate::import`).

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use teleprompt_scene::record::{record_session, Settings, Start};

use crate::import::{draft_session, refuse_to_replace, ImportReport, Session, Words};
use teleprompt_core::Reporter;
use teleprompt_project::project::Project;
use teleprompt_project::registry::{Recording, Registry};

pub struct Record<'a> {
    pub registry: Registry,
    pub reporter: &'a dyn Reporter,
    pub script: &'a Path,
    /// The plugin whose tool records; asciinema when `None`.
    pub with: Option<&'a str>,
    pub model: &'a Path,
    /// A punctuation model's directory, as for `import`.
    pub punctuation: Option<&'a Path>,
    /// ffmpeg's input arguments for the microphone; the platform's default
    /// input when empty.
    pub mic: Vec<String>,
    /// A terminal tool's shell; `$SHELL` when empty.
    pub shell: Vec<String>,
    /// A browser tool's first page.
    pub url: Option<&'a str>,
    pub force: bool,
    /// Where to say how the recording is going, as JSON, for an app that
    /// cannot wait on it.
    pub status: Option<&'a Path>,
    /// Print nothing into the terminal but errors.
    pub quiet: bool,
}

pub fn run_record(r: &Record) -> Result<ImportReport, String> {
    let say = |state: serde_json::Value| {
        if let Some(path) = r.status {
            let partial = path.with_extension("partial");
            let _ = std::fs::write(&partial, state.to_string())
                .and_then(|()| std::fs::rename(&partial, path));
        }
    };
    let result = recorder(r.registry, r.with).and_then(|recorder| {
        check(r, &recorder)?;
        record(r, &recorder, &say)
    });
    say(match &result {
        Ok(report) => serde_json::json!({
            "state": "done",
            "script": report.created,
            "lines": report.lines,
            "blocks": report.blocks,
        }),
        Err(e) => serde_json::json!({ "state": "failed", "error": e }),
    });
    result
}

/// The recorder `with` names, asciinema's by default.
fn recorder(registry: Registry, with: Option<&str>) -> Result<Recording, String> {
    let all = registry.recorders();
    let names: Vec<&str> = all.iter().map(|r| r.plugin).collect();
    let names = names.join(", ");
    let wanted = with.unwrap_or("asciinema");
    all.into_iter()
        .find(|r| r.plugin == wanted)
        .ok_or_else(|| format!("`{wanted}` cannot record a session; these can: {names}"))
}

/// Everything that could stop the draft, checked before anything is
/// recorded rather than after the author has talked for ten minutes.
fn check(r: &Record, recorder: &Recording) -> Result<(), String> {
    refuse_to_replace(r.script, r.force)?;
    if !cfg!(feature = "listen") {
        return Err(
            "this teleprompt was built without a speech recognizer: rebuild it \
                    with `--features listen`"
                .to_string(),
        );
    }
    if !r.model.is_dir() {
        return Err(format!("no speech model at {}", r.model.display()));
    }
    if let Some(dir) = r.punctuation.filter(|d| !d.is_dir()) {
        return Err(format!("no punctuation model at {}", dir.display()));
    }
    if let Some(why) = recorder.unavailable() {
        return Err(format!("cannot record with {}: {why}", recorder.plugin));
    }
    Ok(())
}

fn record(
    r: &Record,
    recorder: &Recording,
    say: &dyn Fn(serde_json::Value),
) -> Result<ImportReport, String> {
    let project = Project::for_script(r.script, r.registry).map_err(|e| e.to_string())?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let stem = r.script.file_stem().unwrap_or_default().to_string_lossy();
    let dir = project
        .root
        .join(".teleprompt/traces")
        .join(format!("{stem}-{stamp}"));
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;

    let voice = dir.join("voice.wav");
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let session = record_session(
        &**recorder,
        &Settings {
            into: &dir,
            start: Start {
                cwd: &cwd,
                shell: &r.shell,
                url: r.url,
            },
            mic: &r.mic,
        },
        &|| {
            say(serde_json::json!({
                "state": "recording",
                "pid": std::process::id(),
                "trace": dir,
            }));
            if !r.quiet {
                let how = if recorder.in_terminal() {
                    "talk as you work, then exit the shell to finish"
                } else {
                    "talk as you work in its window, then close it to finish"
                };
                r.reporter.note(&format!(
                    "recording with {} and the microphone: {how}\r",
                    recorder.plugin
                ));
            }
        },
    )?;

    say(serde_json::json!({ "state": "drafting" }));
    if !r.quiet {
        r.reporter
            .note(&format!("transcribing {}", voice.display()));
    }
    draft_session(&Session {
        registry: r.registry,
        reporter: r.reporter,
        script: r.script,
        recorder,
        recorded: &session.recorded,
        voice: &session.voice,
        words: &Words::Model(r.model),
        offset_ms: session.offset_ms,
        punctuation: r.punctuation,
        force: r.force,
    })
}

/// A recorder, as `record --tools` lists it for an app to offer.
#[derive(Debug, Serialize)]
pub struct Tool {
    pub plugin: &'static str,
    /// Whether the author works in the terminal, or a window of its own.
    pub in_terminal: bool,
    /// Why it cannot record here, if it cannot.
    pub unavailable: Option<String>,
}

/// Every recorder this build has, the default first.
pub fn tools(registry: Registry) -> Vec<Tool> {
    registry
        .recorders()
        .iter()
        .map(|r| Tool {
            plugin: r.plugin,
            in_terminal: r.in_terminal(),
            unavailable: r.unavailable(),
        })
        .collect()
}
