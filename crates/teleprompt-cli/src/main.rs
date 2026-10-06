//! The `teleprompt` binary: the commands, each with its help, and which
//! module runs it. A command's flags and its `run` are in its module under
//! `commands`; what they share is `cli`. What they do is the `teleprompt`
//! crate's.

mod ask;
mod cli;
mod commands;
mod output;

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use cli::{fail, Run, ScriptArgs};
use commands::{
    build, cache, capture, check, dub, edit, import, new, plan, record, serve, setup, translate,
    voice,
};
use output::{exit_code_for, Format, Outcome};

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

#[derive(Subcommand)]
enum Command {
    /// Scaffold a new project
    New(new::Args),
    /// Voices of your own, on a Voicebox server
    Voice(voice::Args),
    /// Install what you need for what you want to do with teleprompt
    ///
    /// In a terminal, with no names, asks what you want to do and installs
    /// what that needs. Teleprompt ships none of it: each tool and model is
    /// under its own license, which this says, and installed with your own
    /// package manager. Name uses (render, terminal, browser, slides,
    /// desktop, prompt, drafts, conversations), scene plugins (vhs,
    /// playwright…) or tools (ffmpeg, speech-model…) to see what they need;
    /// it prints the commands unless --run. Without names outside a
    /// terminal, it reports on all of them.
    ///
    /// It also lists, as a table each, the scene plugins and the voices, or
    /// those named. Scene plugins are built in or installed as programs: named teleprompt-scene-<name>, on PATH or in the
    /// plugins directory ($TELEPROMPT_PLUGINS, or teleprompt/plugins in
    /// your data directory). Each is asked what it needs; one that does not
    /// answer, or whose name a built-in one has, says why it is not used.
    /// Writing one: docs/guide/scene-plugins.md.
    Setup(setup::Args),
    /// Report what the project's caches hold, or shrink them
    ///
    /// Narration and encoded video are both entirely derived: every entry
    /// can be remade from the key that names it, so throwing one away
    /// costs time and nothing else.
    Cache(cache::Args),
    /// Draft a script from something you have: a document, a deck, a
    /// conversation, or a session you recorded while talking
    ///
    /// From a document, prose becomes narration lines with their ids
    /// promoted, shell code blocks become terminal tapes that type the
    /// command and are marked `review=pending` until a human has read
    /// them, and everything else is left as ordinary Markdown for you to
    /// promote by hand. A transcript becomes a line per turn, labelled with
    /// who says it, and so does a recording, transcribed and its voices
    /// told apart, each line speaking its stretch of it (opt-in build).
    ///
    /// From a session (an asciicast recorded with `asciinema rec --stdin`,
    /// asciinema 3: `--capture-input`, or a `vhs record` tape) and the
    /// voice recorded with it, what you said becomes the narration, cut
    /// into lines where you paused, and what you did becomes blocks that
    /// include the recording's parts. Each line is spoken from your voice.
    /// `teleprompt record` records both at once.
    Import(import::Args),
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
    Edit(edit::Args),
    /// Translate a script's narration into another locale
    ///
    /// Writes <script>.<locale>.yaml beside the script, translating only
    /// what is missing or has changed in the English since; read it over,
    /// since it is spoken as written. Compile, dub or build with
    /// `--locale <locale>` to use it. Translates with a model run locally by
    /// Ollama unless [translate] in teleprompt.toml says otherwise.
    Translate(translate::Args),
    /// Record yourself working while you talk, and get a script
    ///
    /// Records with the tool you pick (asciinema, vhs or playwright) and
    /// the microphone until you exit the shell or close the browser, then
    /// drafts <script> from the session as `import` does: what you said is
    /// the narration, spoken from your recording, and what you did is the
    /// recording, saved beside the script and included in parts. The raw
    /// recording is kept in .teleprompt/traces. Needs ffmpeg, the tool,
    /// and a build with `--features listen`.
    Record(record::Args),
    /// Serve the language server on stdin and stdout, for an editor
    ///
    /// Problems as a script is typed, completion of attributes, speakers,
    /// scenes and policies, hover for what a line or block compiles to,
    /// and go to a speaker, scene or included file. docs/guide/editors.md
    /// says how to set an editor up.
    Lsp,
    /// Parse and validate; no side effects, no cost
    Check(ScriptArgs),
    /// Compile the timeline and print it, or compare it with the committed one
    Plan(plan::Args),
    /// Serve the script's prompter: it follows your voice, and plays the video as it will be
    ///
    /// Serves a page on loopback that listens through the microphone and
    /// scrolls to where you are reading, by matching what a local speech
    /// model hears against the script. Nothing leaves the machine. Needs a
    /// build with `--features listen` and a streaming model (see --model).
    ///
    /// V on the page plays the video as it will play: the manifest `dub`
    /// publishes, the artifact an outside consumer reads, so it cannot
    /// drift from what `build` renders. A save while it plays opens on what
    /// moved. With --voice the script's voice reads it, in any build.
    Serve(serve::Args),
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
    Dub(dub::Args),
    /// Record the scenes a build will show
    ///
    /// Runs each scene as one session — its items continue one another —
    /// and keeps a clip for every item that has none. `build` does this on
    /// the way past; this is the same work on its own, for filling a cache
    /// before a render or after editing a tape.
    Capture(capture::Args),
    /// Render the video
    ///
    /// Synthesizes narration, publishes the manifest, and renders it with
    /// ffmpeg. Shots that nothing has captured hold their slot as a slate
    /// — the timing is the scheduled timing either way, and the count of
    /// them is reported.
    Build(build::Args),
}

/// Synchronous on purpose: only the commands that need async build a
/// runtime (docs/design.md#async-boundary).
fn main() -> ExitCode {
    let cli = Cli::parse();
    let format = cli.format;
    let outcome = run(cli.command, format).unwrap_or_else(|failure| fail(format, failure));
    ExitCode::from(exit_code_for(&outcome) as u8)
}

fn run(command: Command, format: Format) -> Run {
    match command {
        Command::New(args) => new::run(args, format),
        Command::Voice(args) => voice::run(args, format),
        Command::Setup(args) => setup::run(args, format),
        Command::Cache(args) => cache::run(args, format),
        Command::Import(args) => import::run(args, format),
        Command::Edit(args) => edit::run(args, format),
        Command::Translate(args) => translate::run(args, format),
        Command::Record(args) => record::run(args, format),
        Command::Lsp => {
            teleprompt_lsp::run_lsp(cli::registry()).map_err(cli::runtime_failure)?;
            Ok(Outcome::Ok)
        }
        Command::Check(args) => check::run(args, format),
        Command::Plan(args) => plan::run(args, format),
        Command::Serve(args) => serve::run(args, format),
        Command::Dub(args) => dub::run(args, format),
        Command::Capture(args) => capture::run(args, format),
        Command::Build(args) => build::run(args, format),
    }
}
