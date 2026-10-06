use std::path::PathBuf;

use teleprompt::serve::*;

use crate::output::{Format, Outcome};

/// `serve`'s arguments.
#[derive(clap::Args)]
pub struct Args {
    /// The script to open; without one, the page opens on its welcome,
    /// which lists the project's scripts and sets teleprompt up
    pub script: Option<std::path::PathBuf>,
    /// The locale to compile for; the project's `locales.source` if not given
    #[arg(long, value_parser = crate::cli::language_tag)]
    pub locale: Option<String>,
    /// Port to listen on; 0 picks a free one
    #[arg(long, default_value_t = 7879)]
    pub port: u16,
    /// Directory of an unpacked sherpa-onnx streaming zipformer model;
    /// defaults to the one `teleprompt setup speech-model` installed
    #[arg(long)]
    pub model: Option<PathBuf>,
    /// Read the script with its voice instead of following yours:
    /// needs no speech model, in any build
    #[arg(long, conflicts_with = "model")]
    pub voice: bool,
}

/// `serve`, following the reader by ear with the speech model named or
/// installed, or with `--voice`, reading the script with its voice.
pub fn run(args: Args, format: Format) -> crate::cli::Run {
    if let Some(script) = &args.script {
        // A script outside any project is said as every command says it.
        crate::cli::project_for(script)?;
    }
    let model = (!args.voice).then(|| {
        args.model
            .or_else(|| teleprompt::setup::speech_model(None).ok())
    });
    run_serve(
        args.script.as_deref(),
        args.locale.as_deref(),
        args.port,
        match &model {
            Some(model) => Ear::Model(model.as_deref()),
            None => Ear::Voice,
        },
        format == Format::Json,
    )?;
    Ok(Outcome::Ok)
}
