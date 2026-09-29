use std::path::{Path, PathBuf};
use std::process::ExitCode;
use teleprompt_core::{BlockId, LineId};

use clap::{Args, Parser, Subcommand};
use serde::Serialize;
use teleprompt_cli::cmd::build;
use teleprompt_cli::cmd::cache;
use teleprompt_cli::cmd::capture as capture_cmd;
use teleprompt_cli::cmd::check::{self, CheckReport};
use teleprompt_cli::cmd::diff as diff_cmd;
use teleprompt_cli::cmd::doctor;
use teleprompt_cli::cmd::dub;
use teleprompt_cli::cmd::from;
use teleprompt_cli::cmd::import::{self, Import, Words};
use teleprompt_cli::cmd::new::{self, NewReport};
use teleprompt_cli::cmd::plan;
use teleprompt_cli::cmd::serve;
use teleprompt_cli::cmd::setup;
use teleprompt_cli::output::{exit_code_for, ErrorReport, Format, Outcome};
use teleprompt_cli::project::Project;
use teleprompt_render::Progress;

#[derive(Parser)]
#[command(
    name = "teleprompt",
    version,
    about = "Compile videos from version-controlled scripts"
)]
struct Cli {
    #[arg(long, value_enum, global = true, default_value = "human")]
    format: Format,

    #[command(subcommand)]
    command: Command,
}

/// The script a command works on, and the locale to compile it for.
#[derive(Args)]
struct ScriptArgs {
    script: PathBuf,
    /// The locale to compile for; the project's `locales.source` if not given
    #[arg(long, value_parser = language_tag)]
    locale: Option<String>,
}

impl ScriptArgs {
    fn locale(&self, project: &Project) -> String {
        self.locale
            .clone()
            .unwrap_or_else(|| check::source_locale(project))
    }
}

/// A `--locale` or `--to` that is a language tag, refused before it names a file.
fn language_tag(s: &str) -> Result<String, String> {
    teleprompt_core::config::locale_problem(s).map_or_else(|| Ok(s.to_string()), Err)
}

/// A frame size and rate that override the script's `output:` block.
#[derive(Args)]
struct FrameArgs {
    /// Frame size, as WIDTHxHEIGHT
    #[arg(long, value_parser = build::parse_resolution)]
    resolution: Option<(u32, u32)>,
    /// Frames per second
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    fps: Option<u32>,
}

/// What `record` records, and where it writes.
#[derive(Args)]
struct RecordArgs {
    /// The script to write, e.g. scripts/tour.md
    #[arg(required_unless_present = "tools")]
    script: Option<PathBuf>,
    /// The tool to record with, by its adapter: asciinema (the default),
    /// vhs or playwright
    #[arg(long)]
    with: Option<String>,
    /// List the tools this build can record with, and whether each is
    /// installed
    #[arg(long)]
    tools: bool,
    /// Directory of an unpacked sherpa-onnx streaming zipformer model;
    /// defaults to the one `teleprompt setup speech-model` installed
    #[arg(long)]
    model: Option<PathBuf>,
    /// Directory of an unpacked sherpa-onnx punctuation model, to give the
    /// narration capitals and punctuation
    #[arg(long)]
    punctuation: Option<PathBuf>,
    /// ffmpeg's input for the microphone, e.g. "-f alsa -i default";
    /// defaults to the system's default input
    #[arg(long, allow_hyphen_values = true, env = "TELEPROMPT_RECORD_MIC")]
    mic: Option<String>,
    /// The page a browser tool starts on
    #[arg(long)]
    url: Option<String>,
    /// Replace the script, its recording and its lines' takes, if they
    /// exist
    #[arg(long)]
    force: bool,
    /// Write how the recording is going to this file, as JSON: recording
    /// (with its pid), drafting, done or failed
    #[arg(long)]
    status: Option<PathBuf>,
    /// Leave the terminal to the shell: say nothing but errors, for an app
    /// that shows the recording's progress itself
    #[arg(long, short)]
    quiet: bool,
    /// A terminal tool's shell instead of $SHELL, with its arguments
    #[arg(last = true)]
    shell: Vec<String>,
}

/// What `import` reads, and where it writes.
#[derive(Args)]
struct ImportArgs {
    /// The recording: an asciicast with keystrokes, or a tape `vhs record`
    /// wrote
    recording: PathBuf,
    /// The tool that made it, by its adapter; found by its extension
    /// otherwise
    #[arg(long)]
    with: Option<String>,
    /// The voice, as a WAV
    #[arg(long)]
    voice: PathBuf,
    /// Where to write the script; defaults to scripts/<recording>.md in the
    /// project
    #[arg(long)]
    out: Option<PathBuf>,
    /// Directory of an unpacked sherpa-onnx streaming zipformer model;
    /// defaults to the one `teleprompt setup speech-model` installed
    #[arg(long)]
    model: Option<PathBuf>,
    /// Timed words as JSON ([{text, start_ms, end_ms}]) instead of --model
    #[arg(long, conflicts_with = "model")]
    words: Option<PathBuf>,
    /// How many milliseconds after the recording the voice recording
    /// started
    #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
    offset_ms: i64,
    /// Directory of an unpacked sherpa-onnx punctuation model, to give the
    /// narration capitals and punctuation
    #[arg(long)]
    punctuation: Option<PathBuf>,
    /// Replace the script, its recording and its lines' takes, if they
    /// exist
    #[arg(long)]
    force: bool,
}

/// One edit `edit` makes, naming blocks and lines by their ids in `plan`.
#[derive(Subcommand)]
enum EditCommand {
    /// Run the block with its line, from the line's WORDth word (0: with
    /// the line)
    Cue {
        block: BlockId,
        #[arg(long)]
        word: usize,
    },
    /// Run the block after its line
    Hold { block: BlockId },
    /// Put the block after another line: held, or cued at --word
    Move {
        block: BlockId,
        #[arg(long)]
        after: LineId,
        #[arg(long)]
        word: Option<usize>,
    },
    /// Make the block's shots BY times longer (above 1) or shorter
    Stretch {
        block: BlockId,
        #[arg(long)]
        by: f64,
    },
    /// Reword the line to what its take was heard to say, keeping the take
    Said { line: LineId },
}

impl EditCommand {
    fn into_edit(self) -> teleprompt_core::edit::Edit {
        use teleprompt_core::edit::Edit;
        match self {
            EditCommand::Cue { block, word } => Edit::Cue { block, word },
            EditCommand::Hold { block } => Edit::Hold { block },
            EditCommand::Move { block, after, word } => Edit::Move { block, after, word },
            EditCommand::Stretch { block, by } => Edit::Stretch { block, by },
            EditCommand::Said { .. } => unreachable!("said is run on its own"),
        }
    }
}

#[derive(Subcommand)]
enum VoiceCommand {
    /// Make a voice from your takes, so the lines you have not recorded
    /// are spoken in your voice. Clone only your own voice, or one you
    /// have permission to
    Clone {
        /// What to call it: `voice.voice` names it
        name: String,
        /// The language of your takes
        #[arg(long, default_value = "en")]
        language: String,
    },
}

#[derive(Subcommand)]
enum Command {
    /// Scaffold a new project
    New { path: PathBuf },
    /// Report the environment teleprompt can see
    Doctor,
    /// Voices of your own, on a Voicebox server
    Voice {
        #[command(subcommand)]
        command: VoiceCommand,
    },
    /// Find, or install, the tools adapters run and the models backends read
    ///
    /// Teleprompt ships none of them: each is under its own license, which
    /// this says, and installed with your own package manager. Name
    /// adapters (vhs, playwright…) or tools (ffmpeg, speech-model…), or
    /// nothing for all of them. Prints the commands unless --run.
    Setup {
        names: Vec<String>,
        /// Run the commands that install what is missing
        #[arg(long)]
        run: bool,
    },
    /// Report what the project's caches hold, or shrink them
    ///
    /// Narration and encoded video are both entirely derived: every entry
    /// can be remade from the key that names it, so throwing one away
    /// costs time and nothing else.
    Cache {
        /// Shrink the encoded-video cache to this many megabytes, least
        /// recently used first. 0 keeps nothing.
        #[arg(long)]
        prune_to_mb: Option<u64>,
    },
    /// Draft a script from a Markdown document you already have
    ///
    /// Prose becomes narration lines with their ids promoted, shell code
    /// blocks become terminal tapes that type the command and are marked
    /// `review=pending` until a human has read them, and everything else is
    /// left as ordinary Markdown for you to promote by hand.
    From {
        doc: PathBuf,
        /// Where to write the draft; defaults to <doc>.teleprompt.md
        #[arg(long)]
        out: Option<PathBuf>,
        /// Read <doc> as a Slidev deck: its speaker notes become the
        /// narration, a paragraph per `[click]` step
        #[arg(long)]
        slidev: bool,
    },
    /// Draft a script from a session you recorded while talking
    ///
    /// What you said becomes the narration, cut into lines where you
    /// paused; what you did becomes blocks that include the recording's
    /// parts, run with the line you were saying or after the one before,
    /// as they were. Each line is then spoken from your recording. Record
    /// the session with `asciinema rec --stdin` (asciinema 3:
    /// `--capture-input`) or `vhs record` and your voice at the same time,
    /// or use `teleprompt record`.
    Import(ImportArgs),
    /// Move or stretch a shot, as a timeline drag does; or reword a line
    /// to what its take says
    ///
    /// Writes the change into the script as the attributes or place a
    /// person would give the block: `policy=concurrent cue="…"`, a move
    /// after another line, `stretch=`. A shot's edit never changes a line;
    /// `said` rewords one, to keep its take. Nothing is written if the
    /// script would then not compile.
    // Left out of `--help`: the apps call it for a drag, and a person
    // writes the attributes.
    #[command(hide = true)]
    Edit {
        script: PathBuf,
        #[command(subcommand)]
        edit: EditCommand,
    },
    /// Translate a script's narration into another locale
    ///
    /// Writes <script>.<locale>.yaml beside the script, translating only
    /// what is missing or has changed in the English since; read it over,
    /// since it is spoken as written. Compile, dub or build with
    /// `--locale <locale>` to use it. Translates with a model run locally by
    /// Ollama unless [translate] in teleprompt.toml says otherwise.
    Translate {
        script: PathBuf,
        /// The locale to translate into, e.g. nl, fr, pt-BR
        #[arg(long, value_parser = language_tag)]
        to: String,
        /// Translate with this provider: ollama, openai, claude or command
        #[arg(long, conflicts_with = "command")]
        provider: Option<String>,
        /// The provider's model
        #[arg(long, conflicts_with = "command")]
        model: Option<String>,
        /// Translate with this shell command: it gets the request as JSON on
        /// stdin and answers {"items": [{"id", "text"}]} on stdout
        #[arg(long)]
        command: Option<String>,
    },
    /// Record yourself working while you talk, and get a script
    ///
    /// Records with the tool you pick (asciinema, vhs or playwright) and
    /// the microphone until you exit the shell or close the browser, then
    /// drafts <script> from the session as `import` does: what you said is
    /// the narration, spoken from your recording, and what you did is the
    /// recording, saved beside the script and included in parts. The raw
    /// recording is kept in .teleprompt/traces. Needs ffmpeg, the tool,
    /// and a build with `--features listen`.
    Record(RecordArgs),
    /// Parse and validate; no side effects, no cost
    Check(ScriptArgs),
    /// Compile the timeline and print it, or compare it with the committed one
    Plan {
        #[command(flatten)]
        args: ScriptArgs,
        /// Compare against the committed timeline instead: print what
        /// changed, and exit 3 if anything did
        #[arg(long)]
        check: bool,
    },
    /// Serve a live preview that opens on the item that changed
    ///
    /// Watches the script, recompiles on save, synthesizes only what the
    /// cache is missing, and serves a preview on loopback. The preview
    /// reads the published narration manifest — the same artifact an
    /// outside consumer reads — so it cannot drift from what `build`
    /// renders.
    Serve {
        #[command(flatten)]
        args: ScriptArgs,
        /// Port to listen on; 0 picks a free one
        #[arg(long, default_value_t = 7878)]
        port: u16,
    },
    /// Show the script as a prompter that follows your voice
    ///
    /// Serves a page on loopback that listens through the microphone and
    /// scrolls to where you are reading, by matching what a local speech
    /// model hears against the script. Nothing leaves the machine. Needs a
    /// build with `--features listen` and a streaming model (see --model).
    Prompt {
        #[command(flatten)]
        args: ScriptArgs,
        /// Port to listen on; 0 picks a free one
        #[arg(long, default_value_t = 7879)]
        port: u16,
        /// Directory of an unpacked sherpa-onnx streaming zipformer model;
        /// defaults to the one `teleprompt setup speech-model` installed
        #[arg(long)]
        model: Option<std::path::PathBuf>,
    },
    /// Synthesize narration and write audio plus a manifest
    ///
    /// Placing a line: convert its own absolute offsets to frames and
    /// subtract — round(start_ms * fps / 1000) and round((start_ms +
    /// duration_ms) * fps / 1000). Never round duration_ms on its own
    /// (rounding error accumulates and drifts audio out of sync by the end
    /// of a long video), and never take the next line's start_ms as this
    /// one's end: consecutive lines may overlap, so that clips the tail
    /// of the speech. A line's own duration_ms is authoritative for its
    /// length.
    Dub {
        #[command(flatten)]
        args: ScriptArgs,
        /// Output root; one self-contained directory is written per locale
        #[arg(long)]
        out: PathBuf,
        /// Compare against the manifest on disk; exit 3 on drift. Leaves
        /// `--out` untouched, but still synthesizes whatever is not already
        /// cached and writes it to the content-addressed cache — that is
        /// what the comparison measures against
        #[arg(long)]
        check: bool,
    },
    /// Record the scenes a build will show
    ///
    /// Runs each scene as one session — its items continue one another —
    /// and keeps a clip for every item that has none. `build` does this on
    /// the way past; this is the same work on its own, for filling a cache
    /// before a render or after editing a tape.
    Capture {
        #[command(flatten)]
        args: ScriptArgs,
        #[command(flatten)]
        frame: FrameArgs,
    },
    /// Render the video
    ///
    /// Synthesizes narration, publishes the manifest, and renders it with
    /// ffmpeg. Shots that nothing has captured hold their slot as a slate
    /// — the timing is the scheduled timing either way, and the count of
    /// them is reported.
    Build {
        #[command(flatten)]
        args: ScriptArgs,
        /// Where to write the video; defaults to build/<script>.<locale>.mp4 in the project
        #[arg(long)]
        out: Option<PathBuf>,
        #[command(flatten)]
        frame: FrameArgs,
        /// Re-encode every frame instead of reusing cached ones
        #[arg(long)]
        no_cache: bool,
        /// Megabytes of encoded video to keep afterwards; 0 keeps nothing
        #[arg(long)]
        cache_max_mb: Option<u64>,
    },
}

/// A render's progress, each percent of it: to a human at a terminal as a
/// line that rewrites itself with a carriage return, which a log cannot
/// take; with `--format json`, as progress events.
fn progress_reporter(format: Format) -> impl FnMut(Progress) {
    use std::io::{IsTerminal, Write};

    let show = format == Format::Human && std::io::stderr().is_terminal();
    let mut last = u64::MAX;
    move |p: Progress| {
        if p.of_ms == 0 {
            return;
        }
        let percent = (p.rendered_ms.min(p.of_ms) * 100) / p.of_ms;
        if percent == last {
            return;
        }
        last = percent;
        if format == Format::Json {
            teleprompt_cli::output::progress(
                "render",
                String::new,
                serde_json::json!({ "done_ms": p.rendered_ms.min(p.of_ms), "of_ms": p.of_ms }),
            );
            return;
        }
        if !show {
            return;
        }
        let mut err = std::io::stderr();
        let _ = write!(err, "\r  rendering  {percent:>3}%");
        if percent == 100 {
            let _ = writeln!(err);
        }
        let _ = err.flush();
    }
}

/// The async runtime for the commands that need one
/// (docs/design.md#async-boundary). Current-thread: the work is waiting on
/// IO, not computing. The error is a bare `String` so each caller can map
/// it into its own runtime-failure variant.
fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start the async runtime: {e}"))
}

/// Reports a failed command the same way whichever command it was: an
/// [`ErrorReport`] on stdout for `--format json`, otherwise each error on
/// stderr.
fn fail(format: Format, outcome: Outcome) -> Outcome {
    let errors = match &outcome {
        Outcome::RuntimeFailure(message) => std::slice::from_ref(message),
        Outcome::ValidationError(errors) => errors.as_slice(),
        _ => &[],
    };
    match format {
        Format::Json => {
            let report = ErrorReport::new(errors.to_vec());
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
        }
        Format::Human => print_errors(errors),
    }
    outcome
}

/// Each error on stderr behind one `error: `: a rendered diagnostic already
/// carries it, and a bare message does not.
fn print_errors(errors: &[String]) {
    for e in errors {
        eprintln!("error: {}", e.strip_prefix("error: ").unwrap_or(e));
    }
}

/// A command's result: `Err` is a failure not yet reported, which
/// [`fail`] reports in the format asked for.
type Run = Result<Outcome, Outcome>;

fn run_setup(format: Format, names: &[String], run: bool) -> Result<Outcome, Outcome> {
    let tools = setup::resolve(names).map_err(runtime_failure)?;
    let setup = setup::Setup::detect();
    let ran = if run {
        setup.install(&tools).map_err(runtime_failure)?
    } else {
        Vec::new()
    };
    let report = setup.report(&tools, ran);
    emit(format, &report, &report.render(names));
    Ok(Outcome::Ok)
}

fn runtime_failure(e: impl ToString) -> Outcome {
    Outcome::RuntimeFailure(e.to_string())
}

/// Prints a report: pretty JSON for `--format json`, otherwise `human`.
fn emit(format: Format, report: &impl Serialize, human: &str) {
    match format {
        Format::Json => println!("{}", serde_json::to_string_pretty(report).unwrap()),
        Format::Human => print!("{human}"),
    }
}

fn warn(warnings: &[String]) {
    for w in warnings {
        eprintln!("warning: {w}");
    }
}

#[cfg(unix)]
fn run_record(format: Format, args: RecordArgs) -> Run {
    use teleprompt_cli::cmd::record::{run_record, tools, Record};
    if args.tools {
        let tools = tools();
        let human: String = tools
            .iter()
            .map(|t| {
                format!(
                    "{:<12}{}\n",
                    t.adapter,
                    t.unavailable.as_deref().unwrap_or("ready")
                )
            })
            .collect();
        emit(format, &tools, &human);
        return Ok(Outcome::Ok);
    }
    let Some(script) = &args.script else {
        unreachable!("clap requires it without --tools");
    };
    let model = &setup::speech_model(args.model.as_deref()).map_err(runtime_failure)?;
    let punctuation = setup::punctuation_model(args.punctuation.as_deref());
    let mic = args.mic.as_deref().map_or_else(Vec::new, |m| {
        m.split_whitespace().map(str::to_string).collect()
    });
    let report = run_record(&Record {
        script,
        with: args.with.as_deref(),
        model,
        punctuation: punctuation.as_deref(),
        mic,
        shell: args.shell,
        url: args.url.as_deref(),
        force: args.force,
        status: args.status.as_deref(),
        quiet: args.quiet,
    })
    .map_err(Outcome::RuntimeFailure)?;
    if !args.quiet {
        emit(format, &report, &report.render());
    }
    Ok(Outcome::Ok)
}

#[cfg(not(unix))]
fn run_record(_: Format, _: RecordArgs) -> Run {
    Err(Outcome::RuntimeFailure(
        "`record` needs a Unix terminal; record with asciinema and use `import`".to_string(),
    ))
}

fn run_translate_cmd(
    format: Format,
    script: &std::path::Path,
    to: &str,
    choice: &teleprompt_cli::cmd::translate::Choice,
) -> Run {
    use teleprompt_cli::cmd::translate::{run_translate, translator, TranslateError};
    let project = project_for(script)?;
    let failed = |e| match e {
        TranslateError::Validation(v) => Outcome::ValidationError(v),
        TranslateError::Runtime(r) => Outcome::RuntimeFailure(r),
    };
    let translator = translator(&project, script, to, choice).map_err(failed)?;
    let report = tokio::runtime::Runtime::new()
        .map_err(|e| Outcome::RuntimeFailure(e.to_string()))?
        .block_on(run_translate(&project, script, to, &translator))
        .map_err(failed)?;
    emit(format, &report, &report.render());
    Ok(Outcome::Ok)
}

fn run_import_cmd(format: Format, args: ImportArgs) -> Run {
    let script = match args.out {
        Some(out) => out,
        None => default_import_script(&args.recording)?,
    };
    let model;
    let words = match &args.words {
        Some(path) => Words::File(path),
        None => {
            model = setup::speech_model(args.model.as_deref()).map_err(runtime_failure)?;
            Words::Model(&model)
        }
    };
    let punctuation = setup::punctuation_model(args.punctuation.as_deref());
    let report = import::run_import(&Import {
        recording: &args.recording,
        with: args.with.as_deref(),
        voice: &args.voice,
        script: &script,
        words,
        offset_ms: args.offset_ms,
        punctuation: punctuation.as_deref(),
        force: args.force,
    })
    .map_err(Outcome::RuntimeFailure)?;
    emit(format, &report, &report.render());
    Ok(Outcome::Ok)
}

/// `scripts/<recording>.md` in the project around the working directory.
fn default_import_script(recording: &std::path::Path) -> Result<PathBuf, Outcome> {
    let project = Project::discover(std::path::Path::new(".")).map_err(runtime_failure)?;
    let stem = recording.file_stem().unwrap_or_default().to_string_lossy();
    Ok(project.root.join("scripts").join(format!("{stem}.md")))
}

fn project_for(script: &std::path::Path) -> Result<Project, Outcome> {
    Project::for_script(script).map_err(runtime_failure)
}

/// Synchronous on purpose: only the commands that need async build a
/// runtime (docs/design.md#async-boundary).
fn main() -> ExitCode {
    let cli = Cli::parse();
    let format = cli.format;
    teleprompt_cli::output::set_progress_format(format);
    let outcome = run(cli.command, format).unwrap_or_else(|failure| fail(format, failure));
    ExitCode::from(exit_code_for(&outcome) as u8)
}

fn run_edit_cmd(format: Format, script: &Path, edit: EditCommand) -> Run {
    let project = project_for(script)?;
    let report = match edit {
        EditCommand::Said { line } => teleprompt_cli::cmd::edit::run_said(&project, script, &line),
        edit => teleprompt_cli::cmd::edit::run_edit(&project, script, &edit.into_edit()),
    }
    .map_err(|e| Outcome::ValidationError(vec![e]))?;
    let human = if report.changed {
        format!("edited {}\n", report.script.display())
    } else {
        "nothing to change\n".to_string()
    };
    emit(format, &report, &human);
    Ok(Outcome::Ok)
}

fn run(command: Command, format: Format) -> Run {
    match command {
        Command::New { path } => {
            let report = NewReport {
                created: new::scaffold(&path).map_err(runtime_failure)?,
            };
            emit(format, &report, &report.render());
            Ok(Outcome::Ok)
        }
        Command::From { doc, out, slidev } => {
            let report = from::run_from(&doc, out, slidev).map_err(runtime_failure)?;
            emit(format, &report, &report.render());
            Ok(Outcome::Ok)
        }
        Command::Import(args) => run_import_cmd(format, args),
        Command::Edit { script, edit } => run_edit_cmd(format, &script, edit),
        Command::Translate {
            script,
            to,
            provider,
            model,
            command,
        } => {
            let choice = teleprompt_cli::cmd::translate::Choice {
                provider: provider.as_deref(),
                model: model.as_deref(),
                command: command.as_deref(),
            };
            run_translate_cmd(format, &script, &to, &choice)
        }
        Command::Record(args) => run_record(format, args),
        Command::Cache { prune_to_mb } => {
            let project = Project::discover(std::path::Path::new(".")).map_err(runtime_failure)?;
            let report = cache::run_cache(&project, prune_to_mb).map_err(runtime_failure)?;
            emit(format, &report, &report.render());
            Ok(Outcome::Ok)
        }
        Command::Doctor => {
            let registry = teleprompt_cli::scene::scenes();
            let report = runtime()
                .map_err(runtime_failure)?
                .block_on(doctor::doctor_report(&registry));
            emit(format, &report, &report.render());
            Ok(Outcome::Ok)
        }
        Command::Setup { names, run } => run_setup(format, &names, run),
        Command::Voice {
            command: VoiceCommand::Clone { name, language },
        } => {
            let project = Project::discover(std::path::Path::new(".")).map_err(runtime_failure)?;
            let report = runtime()
                .map_err(runtime_failure)?
                .block_on(teleprompt_cli::cmd::voice::run_clone(
                    &project, &name, &language,
                ))
                .map_err(runtime_failure)?;
            emit(format, &report, &report.render());
            Ok(Outcome::Ok)
        }
        Command::Check(args) => run_check(format, &args),
        Command::Plan { args, check: false } => run_plan(format, &args),
        Command::Plan { args, check: true } => run_plan_check(format, &args),
        Command::Prompt { args, port, model } => {
            let project = project_for(&args.script)?;
            teleprompt_cli::cmd::prompt::run_prompt(
                &project,
                &args.script,
                &args.locale(&project),
                port,
                model.or_else(|| setup::speech_model(None).ok()).as_deref(),
                format,
            )?;
            Ok(Outcome::Ok)
        }
        Command::Serve { args, port } => run_serve(&args, port),
        Command::Build {
            args,
            out,
            frame,
            no_cache,
            cache_max_mb,
        } => run_build(format, &args, out, &frame, no_cache, cache_max_mb),
        Command::Capture { args, frame } => run_capture(format, &args, &frame),
        Command::Dub { args, out, check } => run_dub(format, &args, &out, check),
    }
}

fn run_plan_check(format: Format, args: &ScriptArgs) -> Run {
    let project = project_for(&args.script)?;
    let d = diff_cmd::run_diff(&project, &args.script, &args.locale(&project))
        .map_err(Outcome::ValidationError)?;
    emit(format, &d, &format!("{}\n", d.render()));
    Ok(if !d.is_empty() {
        Outcome::Drift
    } else {
        Outcome::Ok
    })
}

fn run_serve(args: &ScriptArgs, port: u16) -> Run {
    let project = project_for(&args.script)?;
    let locale = args.locale(&project);
    runtime()
        .map_err(runtime_failure)?
        .block_on(serve::run_serve(&project, &args.script, &locale, port))?;
    Ok(Outcome::Ok)
}

/// `check` reports its own failure: its JSON report has room for the errors.
fn run_check(format: Format, args: &ScriptArgs) -> Run {
    let project = project_for(&args.script)?;
    match check::run_check(&project, &args.script, &args.locale(&project)) {
        Ok(warnings) => {
            warn(&warnings);
            let report = CheckReport {
                ok: true,
                warnings,
                errors: Vec::new(),
            };
            emit(format, &report, "ok\n");
            Ok(Outcome::Ok)
        }
        Err(errors) => {
            match format {
                Format::Json => {
                    let report = CheckReport {
                        ok: false,
                        warnings: Vec::new(),
                        errors: errors.clone(),
                    };
                    println!("{}", serde_json::to_string_pretty(&report).unwrap())
                }
                Format::Human => print_errors(&errors),
            }
            Ok(Outcome::ValidationError(errors))
        }
    }
}

fn run_plan(format: Format, args: &ScriptArgs) -> Run {
    let project = project_for(&args.script)?;
    let out = plan::run_plan(&project, &args.script, &args.locale(&project))
        .map_err(Outcome::ValidationError)?;
    emit(format, &out.timeline, &plan::render_plan(&out));

    let narrated = out
        .timeline
        .entries
        .iter()
        .filter_map(|e| e.narration.as_ref());
    let total = narrated.clone().count();
    let estimated = narrated
        .filter(|n| n.duration_source == teleprompt_core::DurationSource::Estimated)
        .count();
    if estimated > 0 {
        eprintln!(
            "warning: {estimated} of {total} narration durations are \
             estimated; run `teleprompt dub` to measure them before \
             committing this timeline"
        );
    }
    Ok(Outcome::Ok)
}

fn run_build(
    format: Format,
    args: &ScriptArgs,
    out: Option<PathBuf>,
    frame: &FrameArgs,
    no_cache: bool,
    cache_max_mb: Option<u64>,
) -> Run {
    let project = project_for(&args.script)?;
    let size = frame.resolution;
    let mut options = build::BuildOptions::defaults(&project, &args.script, &args.locale(&project));
    if let Some(path) = out {
        options.out = path;
    }
    if size.is_some() {
        options.resolution = size;
    }
    options.fps = frame.fps.or(options.fps);
    if no_cache {
        options.compose_dir = None;
    }
    options.cache_max_mb = cache_max_mb.unwrap_or(options.cache_max_mb);

    let mut show = progress_reporter(format);
    let renderer = build::renderer(&options);
    let report = runtime()
        .map_err(runtime_failure)?
        .block_on(build::run_build_with(
            &renderer,
            &project,
            &args.script,
            &args.locale(&project),
            &options,
            &mut show,
        ))?;
    warn(&report.warnings);
    emit(format, &report, &report.render());
    Ok(Outcome::Ok)
}

fn run_capture(format: Format, args: &ScriptArgs, frame: &FrameArgs) -> Run {
    let project = project_for(&args.script)?;
    let size = frame.resolution;
    let mut progress = |p: teleprompt_capture::Progress| {
        teleprompt_cli::output::progress(
            "capture",
            || format!("  [{}/{}] {} {}", p.done, p.of, p.scene, p.shot),
            serde_json::json!({ "done": p.done, "of": p.of, "scene": p.scene, "shot": p.shot }),
        );
    };
    let report = runtime()
        .map_err(runtime_failure)?
        .block_on(capture_cmd::capture_script(
            &project,
            &args.script,
            &args.locale(&project),
            size,
            frame.fps,
            &mut progress,
        ))?;
    warn(&report.warnings);
    emit(format, &report, &report.render());
    Ok(Outcome::Ok)
}

fn run_dub(format: Format, args: &ScriptArgs, out: &std::path::Path, check: bool) -> Run {
    let project = project_for(&args.script)?;
    let result = runtime().map_err(runtime_failure)?.block_on(dub::run_dub(
        &project,
        &args.script,
        &args.locale(&project),
        out,
        check,
    ))?;
    warn(&result.warnings);
    match &result.drift {
        Some(d) => emit(format, d, &d.render()),
        None => emit(format, &result.manifest, &dub::render_dub(&result)),
    }
    Ok(if result.drift.as_ref().is_some_and(|d| !d.is_empty()) {
        Outcome::Drift
    } else {
        Outcome::Ok
    })
}
