use teleprompt_draft::record::*;

/// What `record` records, and where it writes.
#[derive(clap::Args)]
pub struct Args {
    /// The script to write, e.g. scripts/tour.md
    #[arg(required_unless_present = "tools")]
    pub script: Option<std::path::PathBuf>,
    /// The tool to record with, by its scene plugin: asciinema (the default),
    /// vhs or playwright
    #[arg(long)]
    pub with: Option<String>,
    /// List the tools this build can record with, and whether each is
    /// installed
    #[arg(long)]
    pub tools: bool,
    /// Directory of an unpacked sherpa-onnx streaming zipformer model;
    /// defaults to the one `teleprompt setup speech-model` installed
    #[arg(long)]
    pub model: Option<std::path::PathBuf>,
    /// Directory of an unpacked sherpa-onnx punctuation model, to give the
    /// narration capitals and punctuation
    #[arg(long)]
    pub punctuation: Option<std::path::PathBuf>,
    /// ffmpeg's input for the microphone, e.g. "-f alsa -i default";
    /// defaults to the system's default input
    #[arg(long, allow_hyphen_values = true, env = "TELEPROMPT_RECORD_MIC")]
    pub mic: Option<String>,
    /// The page a browser tool starts on
    #[arg(long)]
    pub url: Option<String>,
    /// Replace the script, its recording and its lines' takes, if they
    /// exist
    #[arg(long)]
    pub force: bool,
    /// Write how the recording is going to this file, as JSON: recording
    /// (with its pid), drafting, done or failed
    #[arg(long)]
    pub status: Option<std::path::PathBuf>,
    /// Leave the terminal to the shell: say nothing but errors, for an app
    /// that shows the recording's progress itself
    #[arg(long, short)]
    pub quiet: bool,
    /// A terminal tool's shell instead of $SHELL, with its arguments
    #[arg(last = true)]
    pub shell: Vec<String>,
}

pub fn run(args: Args, format: crate::output::Format) -> crate::cli::Run {
    use crate::cli::{emit, emit_data};
    use crate::output::Outcome;
    use teleprompt_setup as setup;
    if args.tools {
        let tools = tools(crate::cli::registry());
        let human: String = tools
            .iter()
            .map(|t| {
                format!(
                    "{:<12}{}\n",
                    t.plugin,
                    t.unavailable.as_deref().unwrap_or("ready")
                )
            })
            .collect();
        emit_data(format, &tools, &human);
        return Ok(Outcome::Ok);
    }
    let Some(script) = &args.script else {
        unreachable!("clap requires it without --tools");
    };
    let reporter = crate::output::Terminal::new(format);
    let model = &setup::speech_model(args.model.as_deref(), &reporter)?;
    let punctuation = setup::punctuation_model(args.punctuation.as_deref());
    let mic = args.mic.as_deref().map_or_else(Vec::new, |m| {
        m.split_whitespace().map(str::to_string).collect()
    });
    let report = run_record(&Record {
        registry: crate::cli::registry(),
        reporter: &reporter,
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
    })?;
    if !args.quiet {
        emit(format, &report, &report.render());
    }
    Ok(Outcome::Ok)
}
