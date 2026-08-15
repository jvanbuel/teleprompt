use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use teleprompt_cli::cmd::doctor;
use teleprompt_cli::cmd::new::{self, NewReport};
use teleprompt_cli::output::{exit_code_for, Format, Outcome};
use teleprompt_scene::SceneRegistry;

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
    New { path: PathBuf },
    /// Report the environment teleprompt can see
    Doctor,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let registry = SceneRegistry::with_builtins();

    let outcome = match cli.command {
        Command::New { path } => match new::scaffold(&path) {
            Ok(files) => {
                let report = NewReport { created: files };
                match cli.format {
                    Format::Json => println!("{}", serde_json::to_string_pretty(&report).unwrap()),
                    Format::Human => print!("{}", report.render()),
                }
                Outcome::Ok
            }
            Err(e) => {
                eprintln!("error: {e}");
                Outcome::RuntimeFailure(e.to_string())
            }
        },
        Command::Doctor => {
            let report = doctor::doctor_report(&registry);
            match cli.format {
                Format::Json => println!("{}", serde_json::to_string_pretty(&report).unwrap()),
                Format::Human => print!("{}", report.render()),
            }
            Outcome::Ok
        }
    };

    ExitCode::from(exit_code_for(&outcome) as u8)
}
