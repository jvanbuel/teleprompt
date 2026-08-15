use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use teleprompt_cli::cmd::check::{self, CheckReport};
use teleprompt_cli::cmd::diff as diff_cmd;
use teleprompt_cli::cmd::doctor;
use teleprompt_cli::cmd::dub;
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
    /// Synthesize narration and write audio plus a manifest
    ///
    /// Placing a segment: convert its own absolute offsets to frames and
    /// subtract — round(start_ms * fps / 1000) and round((start_ms +
    /// duration_ms) * fps / 1000). Never round duration_ms on its own
    /// (rounding error accumulates and drifts audio out of sync by the end
    /// of a long video), and never take the next segment's start_ms as this
    /// one's end: consecutive segments may overlap, so that clips the tail
    /// of the speech. A segment's own duration_ms is authoritative for its
    /// length.
    Dub {
        script: PathBuf,
        #[arg(long, default_value = "en")]
        locale: String,
        /// Output root; one self-contained directory is written per locale
        #[arg(long)]
        out: PathBuf,
        /// Compare against the manifest on disk and write nothing; exit 3 on drift
        #[arg(long)]
        check: bool,
        /// Treat a voice-tier downgrade as fatal; exit 4
        #[arg(long)]
        strict_voice: bool,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
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
        Command::Dub {
            script,
            locale,
            out,
            check,
            strict_voice,
        } => match Project::for_script(&script) {
            Err(e) => {
                eprintln!("error: {e}");
                Outcome::RuntimeFailure(e.to_string())
            }
            Ok(project) => match dub::run_dub(&project, &script, &locale, &out, check).await {
                Ok(result) => {
                    for w in &result.warnings {
                        eprintln!("warning: {w}");
                    }
                    match (&result.drift, cli.format) {
                        (Some(d), Format::Json) => {
                            println!("{}", serde_json::to_string_pretty(d).unwrap())
                        }
                        (Some(d), Format::Human) => print!("{}", d.render()),
                        (None, Format::Json) => {
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&result.manifest).unwrap()
                            )
                        }
                        (None, Format::Human) => print!("{}", dub::render_dub(&result)),
                    }
                    // A downgrade is reported either way — an author should
                    // hear that their `recorded` script was machine-read
                    // whether or not they asked for it to be fatal.
                    if !result.downgrades.is_empty() {
                        eprintln!(
                            "voice downgraded on {} segment(s):",
                            result.downgrades.len()
                        );
                        eprint!("{}", dub::render_downgrades(&result.downgrades));
                    }

                    // Checked ahead of drift: a downgrade means the audio is
                    // not what the script asked for, which is true whether or
                    // not the committed manifest happens to agree with it.
                    if strict_voice && !result.downgrades.is_empty() {
                        Outcome::VoiceDowngrade
                    } else {
                        match &result.drift {
                            Some(d) if !d.is_empty() => Outcome::Drift,
                            _ => Outcome::Ok,
                        }
                    }
                }
                Err(dub::DubError::Validation(errors)) => {
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
                Err(dub::DubError::Runtime(e)) => {
                    eprintln!("error: {e}");
                    Outcome::RuntimeFailure(e)
                }
            },
        },
    };

    ExitCode::from(exit_code_for(&outcome) as u8)
}
