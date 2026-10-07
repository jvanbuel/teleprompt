//! What every command shares on the command line: the script and locale
//! it works on, a frame size, how it reports in either format, and how a
//! failure is told. Each command's own arguments and `run` are in its
//! module under `commands`; `main` lists the commands.

use std::path::{Path, PathBuf};

use clap::Args;
use serde::Serialize;

use crate::output::{ErrorReport, Format, Outcome};
use teleprompt_project::project::Project;

/// A command's result: `Err` is a failure not yet reported, which
/// [`fail`] reports in the format asked for.
pub type Run = Result<Outcome, Outcome>;

/// The script a command works on, and the locale to compile it for.
#[derive(Args)]
pub struct ScriptArgs {
    pub script: PathBuf,
    /// The locale to compile for; the project's `locales.source` if not given
    #[arg(long, value_parser = language_tag)]
    pub locale: Option<String>,
}

impl ScriptArgs {
    pub fn locale(&self, project: &Project) -> String {
        self.locale
            .clone()
            .unwrap_or_else(|| project.source_locale())
    }

    /// The project the script is in.
    pub fn project(&self) -> Result<Project, Outcome> {
        project_for(&self.script)
    }

    /// The script, in its project, for the locale asked for.
    pub fn open(&self) -> Result<teleprompt_project::project::Script, Outcome> {
        let project = self.project()?;
        let locale = self.locale(&project);
        Ok(teleprompt_project::project::Script::open(
            project,
            &self.script,
            locale,
        ))
    }
}

/// A `--locale` or `--to` that is a language tag, refused before it names a file.
pub fn language_tag(s: &str) -> Result<String, String> {
    teleprompt_script::config::locale_problem(s).map_or_else(|| Ok(s.to_string()), Err)
}

/// A frame size and rate that override the script's `output:` block.
#[derive(Args, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameArgs {
    /// Frame size, as WIDTHxHEIGHT
    #[arg(long, value_parser = teleprompt_project::build::parse_resolution)]
    pub resolution: Option<(u32, u32)>,
    /// Frames per second
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    pub fps: Option<u32>,
}

impl From<FrameArgs> for teleprompt_project::build::FrameOverride {
    fn from(args: FrameArgs) -> Self {
        Self {
            resolution: args.resolution,
            fps: args.fps,
        }
    }
}

/// What this build has: every scene plugin and voice, found once.
pub fn registry() -> teleprompt_project::registry::Registry {
    crate::registry::registry()
}

/// The project `script` is in.
pub fn project_for(script: &Path) -> Result<Project, Outcome> {
    Project::for_script(script, registry()).map_err(runtime_failure)
}

/// The project around the working directory.
pub fn project_here() -> Result<Project, Outcome> {
    Project::discover(Path::new("."), registry()).map_err(runtime_failure)
}

pub fn runtime_failure(e: impl ToString) -> Outcome {
    Outcome::RuntimeFailure(e.to_string())
}

/// The async runtime for the commands that need one
/// (docs/design.md#async-boundary). Current-thread: the work is waiting on
/// IO, not computing.
pub fn runtime() -> Result<tokio::runtime::Runtime, Outcome> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| runtime_failure(format!("cannot start the async runtime: {e}")))
}

/// Prints a report: pretty JSON for `--format json`, otherwise `human`.
/// As JSON it says `ok`, the one key every report has for a script to
/// test: whether the command exits 0.
pub fn emit(format: Format, report: &impl Serialize, human: &str) {
    emit_ok(format, report, human, true);
}

pub fn emit_ok(format: Format, report: &impl Serialize, human: &str, ok: bool) {
    let mut value = serde_json::to_value(report).unwrap_or_else(|e| unserializable(&e));
    if let serde_json::Value::Object(fields) = &mut value {
        fields.entry("ok").or_insert(ok.into());
    }
    emit_data(format, &value, human);
}

/// A document, printed as it is written to disk or read by other tools:
/// a timeline, a manifest, a list.
pub fn emit_data(format: Format, data: &impl Serialize, human: &str) {
    match format {
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(data).unwrap_or_else(|e| unserializable(&e).to_string())
        ),
        Format::Human => print!("{human}"),
    }
}

/// What `--format json` says when a report cannot be written as JSON (a map
/// with keys that are not strings, say): an error report a script can still
/// read, not a panic halfway through the output.
fn unserializable(e: &serde_json::Error) -> serde_json::Value {
    serde_json::json!({
        "ok": false,
        "errors": [format!("cannot write the report as JSON: {e}")],
    })
}

pub fn warn(warnings: &[String]) {
    for w in warnings {
        eprintln!("warning: {w}");
    }
}

/// Reports a failed command the same way whichever command it was: an
/// [`ErrorReport`] on stdout for `--format json`, otherwise each error on
/// stderr.
pub fn fail(format: Format, outcome: Outcome) -> Outcome {
    let errors = match &outcome {
        Outcome::RuntimeFailure(message) => std::slice::from_ref(message),
        Outcome::ValidationError(errors) => errors.as_slice(),
        _ => &[],
    };
    match format {
        Format::Json => {
            let report = ErrorReport::new(errors.to_vec());
            let json = serde_json::to_string_pretty(&report)
                .expect("BUG: an ErrorReport is a bool and strings, which always serialize");
            println!("{json}");
        }
        Format::Human => print_errors(errors),
    }
    outcome
}

/// Each error on stderr behind one `error: `: a rendered diagnostic already
/// carries it, and a bare message does not.
pub fn print_errors(errors: &[String]) {
    for e in errors {
        eprintln!("error: {}", e.strip_prefix("error: ").unwrap_or(e));
    }
}
