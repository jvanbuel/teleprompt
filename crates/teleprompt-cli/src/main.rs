use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use teleprompt_cli::cmd::check::{self, CheckReport};
use teleprompt_cli::cmd::diff as diff_cmd;
use teleprompt_cli::cmd::doctor;
use teleprompt_cli::cmd::new::{self, NewReport};
use teleprompt_cli::cmd::plan;
use teleprompt_cli::output::{exit_code_for, ErrorReport, Format, Outcome};
use teleprompt_cli::project::Project;
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
    /// Parse and validate; no side effects, no cost
    Check {
        script: PathBuf,
        #[arg(long, default_value = "en")]
        locale: String,
    },
    /// Compile the timeline and print it
    Plan {
        script: PathBuf,
        #[arg(long, default_value = "en")]
        locale: String,
    },
    /// Compare against the committed timeline
    Diff {
        script: PathBuf,
        #[arg(long, default_value = "en")]
        locale: String,
        /// Exit 3 when the timeline has drifted
        #[arg(long)]
        exit_code: bool,
    },
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
        Command::Check { script, locale } => match Project::for_script(&script) {
            Err(e) => {
                eprintln!("error: {e}");
                Outcome::RuntimeFailure(e.to_string())
            }
            Ok(project) => match check::run_check(&project, &script, &locale) {
                Ok(warnings) => {
                    for w in &warnings {
                        eprintln!("warning: {w}");
                    }
                    let report = CheckReport {
                        ok: true,
                        warnings,
                        errors: Vec::new(),
                    };
                    match cli.format {
                        Format::Json => {
                            println!("{}", serde_json::to_string_pretty(&report).unwrap())
                        }
                        Format::Human => println!("ok"),
                    }
                    Outcome::Ok
                }
                Err(errors) => {
                    match cli.format {
                        Format::Json => {
                            let report = CheckReport {
                                ok: false,
                                warnings: Vec::new(),
                                errors: errors.clone(),
                            };
                            println!("{}", serde_json::to_string_pretty(&report).unwrap())
                        }
                        Format::Human => {
                            for e in &errors {
                                eprintln!("{e}");
                            }
                        }
                    }
                    Outcome::ValidationError(errors)
                }
            },
        },
        Command::Plan { script, locale } => match Project::for_script(&script) {
            Err(e) => {
                eprintln!("error: {e}");
                Outcome::RuntimeFailure(e.to_string())
            }
            Ok(project) => match plan::run_plan(&project, &script, &locale) {
                Ok(out) => {
                    match cli.format {
                        Format::Json => {
                            println!("{}", serde_json::to_string_pretty(&out.timeline).unwrap())
                        }
                        Format::Human => print!("{}", plan::render_plan(&out)),
                    }
                    Outcome::Ok
                }
                Err(errors) => {
                    match cli.format {
                        Format::Json => {
                            let report = ErrorReport::new(errors.clone());
                            println!("{}", serde_json::to_string_pretty(&report).unwrap())
                        }
                        Format::Human => {
                            for e in &errors {
                                eprintln!("{e}");
                            }
                        }
                    }
                    Outcome::ValidationError(errors)
                }
            },
        },
        Command::Diff {
            script,
            locale,
            exit_code,
        } => match Project::for_script(&script) {
            Err(e) => {
                eprintln!("error: {e}");
                Outcome::RuntimeFailure(e.to_string())
            }
            Ok(project) => match diff_cmd::run_diff(&project, &script, &locale) {
                Ok(d) => {
                    match cli.format {
                        Format::Json => println!("{}", serde_json::to_string_pretty(&d).unwrap()),
                        Format::Human => println!("{}", d.render()),
                    }
                    if exit_code && !d.is_empty() {
                        Outcome::Drift
                    } else {
                        Outcome::Ok
                    }
                }
                Err(errors) => {
                    match cli.format {
                        Format::Json => {
                            let report = ErrorReport::new(errors.clone());
                            println!("{}", serde_json::to_string_pretty(&report).unwrap())
                        }
                        Format::Human => {
                            for e in &errors {
                                eprintln!("{e}");
                            }
                        }
                    }
                    Outcome::ValidationError(errors)
                }
            },
        },
    };

    ExitCode::from(exit_code_for(&outcome) as u8)
}
