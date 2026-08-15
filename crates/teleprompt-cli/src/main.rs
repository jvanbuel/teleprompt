use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use teleprompt_cli::cmd::{doctor, new};
use teleprompt_cli::output::Format;
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

    match cli.command {
        Command::New { path } => match new::scaffold(&path) {
            Ok(files) => {
                match cli.format {
                    Format::Json => println!(
                        "{}",
                        serde_json::json!({
                            "created": files.iter().map(|p| p.display().to_string()).collect::<Vec<_>>()
                        })
                    ),
                    Format::Human => {
                        for f in files {
                            println!("created {}", f.display());
                        }
                    }
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::from(1)
            }
        },
        Command::Doctor => {
            let report = doctor::doctor_report(&registry);
            match cli.format {
                Format::Json => println!("{}", serde_json::to_string_pretty(&report).unwrap()),
                Format::Human => print!("{}", report.render()),
            }
            ExitCode::SUCCESS
        }
    }
}
