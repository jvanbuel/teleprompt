use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use teleprompt_cli::cmd::build;
use teleprompt_cli::cmd::cache;
use teleprompt_cli::cmd::capture as capture_cmd;
use teleprompt_cli::cmd::check::{self, CheckReport};
use teleprompt_cli::cmd::diff as diff_cmd;
use teleprompt_cli::cmd::doctor;
use teleprompt_cli::cmd::dub;
use teleprompt_cli::cmd::from;
use teleprompt_cli::cmd::new::{self, NewReport};
use teleprompt_cli::cmd::plan;
use teleprompt_cli::cmd::serve;
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

#[derive(Subcommand)]
enum Command {
    /// Scaffold a new project
    New { path: PathBuf },
    /// Report the environment teleprompt can see
    Doctor,
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
    },
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
    /// Serve a live preview that opens on the item that changed
    ///
    /// Watches the script, recompiles on save, synthesizes only what the
    /// cache is missing, and serves a preview on loopback. The preview
    /// reads the published narration manifest — the same artifact an
    /// outside consumer reads — so it cannot drift from what `build`
    /// renders.
    Serve {
        script: PathBuf,
        #[arg(long, default_value = "en")]
        locale: String,
        /// Port to listen on; 0 picks a free one
        #[arg(long, default_value_t = 7878)]
        port: u16,
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
        script: PathBuf,
        #[arg(long, default_value = "en")]
        locale: String,
        /// Output root; one self-contained directory is written per locale
        #[arg(long)]
        out: PathBuf,
        /// Compare against the manifest on disk; exit 3 on drift. Leaves
        /// `--out` untouched, but still synthesizes whatever is not already
        /// cached and writes it to the content-addressed cache — that is
        /// what the comparison measures against
        #[arg(long)]
        check: bool,
        /// Treat a voice-tier downgrade as fatal; exit 4
        #[arg(long)]
        strict_voice: bool,
    },
    /// Record the scenes a build will show
    ///
    /// Runs each scene as one session — its items continue one another —
    /// and keeps a clip for every item that has none. `build` does this on
    /// the way past; this is the same work on its own, for filling a cache
    /// before a render or after editing a tape.
    Capture {
        script: PathBuf,
        #[arg(long, default_value = "en")]
        locale: String,
        /// Frame size, as WIDTHxHEIGHT
        #[arg(long)]
        resolution: Option<String>,
        /// Frames per second
        #[arg(long)]
        fps: Option<u32>,
    },
    /// Render the video
    ///
    /// Synthesizes narration, publishes the manifest, and renders it with
    /// ffmpeg. Beats that nothing has captured hold their slot as a slate
    /// — the timing is the scheduled timing either way, and the count of
    /// them is reported.
    Build {
        script: PathBuf,
        #[arg(long, default_value = "en")]
        locale: String,
        /// Where to write the video; defaults to out/<locale>/<script>.mp4
        #[arg(long)]
        out: Option<PathBuf>,
        /// Frame size, as WIDTHxHEIGHT
        #[arg(long)]
        resolution: Option<String>,
        /// Frames per second
        #[arg(long)]
        fps: Option<u32>,
        /// Re-encode every frame instead of reusing cached ones
        #[arg(long)]
        no_cache: bool,
        /// Megabytes of encoded video to keep afterwards; 0 keeps nothing
        #[arg(long)]
        cache_max_mb: Option<u64>,
    },
}

/// Where a render's progress goes.
///
/// Nothing, unless a human is watching a terminal in human format: the
/// line rewrites itself with a carriage return, which a log file records
/// as one enormous line and a JSON consumer cannot parse at all.
fn progress_reporter(format: Format) -> impl FnMut(Progress) {
    use std::io::{IsTerminal, Write};

    let show = format == Format::Human && std::io::stderr().is_terminal();
    let mut last = u64::MAX;
    move |p: Progress| {
        if !show || p.of_ms == 0 {
            return;
        }
        let percent = (p.rendered_ms.min(p.of_ms) * 100) / p.of_ms;
        if percent == last {
            return;
        }
        last = percent;
        let mut err = std::io::stderr();
        let _ = write!(err, "\r  rendering  {percent:>3}%");
        if percent == 100 {
            let _ = writeln!(err);
        }
        let _ = err.flush();
    }
}

/// The async runtime, built for `dub` and `doctor` and nothing else.
///
/// Current-thread rather than multi-thread: neither command awaits more
/// than one thing at a time, so worker threads have nothing to do. Spec
/// §7.2's bounded concurrent fan-out would want `new_multi_thread` back for
/// `dub` — which is why the `rt-multi-thread` feature is still declared
/// rather than trimmed away.
///
/// Its error is a bare `String`, not `dub::DubError`: `doctor` has no
/// `DubError` channel of its own, so a shared helper cannot return one
/// without lying about where the error came from. `dub`'s call site maps it
/// into `DubError::Runtime` instead, which is where that variant already
/// says a runtime failure belongs — exit 1, not a validation error, taking
/// the same road as any other `dub` failure instead of panicking out
/// through an exit code `exit_code_for` cannot issue.
fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start the async runtime: {e}"))
}

/// Synchronous on purpose. `#[tokio::main]` started a multi-thread runtime
/// — a worker thread per core — for every subcommand, including
/// `check`/`plan`/`diff`/`new`, none of which ever await. `dub` and
/// `doctor` each build their own runtime in their own arm, which is the
/// only place async is reachable from.
fn main() -> ExitCode {
    let cli = Cli::parse();
    let registry = teleprompt_cli::scene::scenes();

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
        Command::From { doc, out } => match from::run_from(&doc, out) {
            Ok(report) => {
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
        Command::Cache { prune_to_mb } => match Project::discover(std::path::Path::new(".")) {
            Err(e) => {
                eprintln!("error: {e}");
                Outcome::RuntimeFailure(e.to_string())
            }
            Ok(project) => match cache::run_cache(&project, prune_to_mb) {
                Ok(report) => {
                    match cli.format {
                        Format::Json => {
                            println!("{}", serde_json::to_string_pretty(&report).unwrap())
                        }
                        Format::Human => print!("{}", report.render()),
                    }
                    Outcome::Ok
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    Outcome::RuntimeFailure(e.to_string())
                }
            },
        },
        Command::Doctor => match runtime() {
            Ok(rt) => {
                let report = rt.block_on(doctor::doctor_report(&registry));
                match cli.format {
                    Format::Json => {
                        println!("{}", serde_json::to_string_pretty(&report).unwrap())
                    }
                    Format::Human => print!("{}", report.render()),
                }
                Outcome::Ok
            }
            Err(e) => {
                eprintln!("error: {e}");
                Outcome::RuntimeFailure(e)
            }
        },
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
                    let estimated = out
                        .timeline
                        .entries
                        .iter()
                        .filter(|e| {
                            e.narration
                                .as_ref()
                                .is_some_and(|n| n.duration_source == "estimated")
                        })
                        .count();
                    if estimated > 0 {
                        let total = out
                            .timeline
                            .entries
                            .iter()
                            .filter(|e| e.narration.is_some())
                            .count();
                        eprintln!(
                            "warning: {estimated} of {total} narration durations are \
                             estimated; run `teleprompt dub` to measure them before \
                             committing this timeline"
                        );
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
        Command::Serve {
            script,
            locale,
            port,
        } => match Project::for_script(&script) {
            Err(e) => {
                eprintln!("error: {e}");
                Outcome::RuntimeFailure(e.to_string())
            }
            Ok(project) => {
                match runtime()
                    .map_err(serve::ServeError::Runtime)
                    .and_then(|rt| rt.block_on(serve::run_serve(&project, &script, &locale, port)))
                {
                    Ok(()) => Outcome::Ok,
                    Err(serve::ServeError::Validation(errors)) => {
                        for e in &errors {
                            eprintln!("{e}");
                        }
                        Outcome::ValidationError(errors)
                    }
                    Err(serve::ServeError::Runtime(e)) => {
                        eprintln!("error: {e}");
                        Outcome::RuntimeFailure(e)
                    }
                }
            }
        },
        Command::Build {
            script,
            locale,
            out,
            resolution,
            fps,
            no_cache,
            cache_max_mb,
        } => match Project::for_script(&script) {
            Err(e) => {
                eprintln!("error: {e}");
                Outcome::RuntimeFailure(e.to_string())
            }
            Ok(project) => {
                let size = resolution.as_deref().map(build::parse_resolution);
                match size {
                    Some(Err(e)) => {
                        eprintln!("error: {e}");
                        Outcome::RuntimeFailure(e)
                    }
                    size => {
                        let mut options = build::BuildOptions::defaults(&project, &script, &locale);
                        if let Some(path) = out {
                            options.out = path;
                        }
                        if let Some(Ok(size)) = size {
                            options.resolution = Some(size);
                        }
                        options.fps = fps.or(options.fps);
                        if no_cache {
                            options.compose_dir = None;
                        }
                        options.cache_max_mb = cache_max_mb.unwrap_or(options.cache_max_mb);
                        // Progress goes to stderr, and only to a terminal: a
                        // carriage-returned percentage is for a human
                        // watching, and in a CI log it is the same line a
                        // few hundred times.
                        let mut show = progress_reporter(cli.format);
                        match runtime()
                            .map_err(build::BuildError::Runtime)
                            .and_then(|rt| {
                                let renderer = build::renderer(&options);
                                rt.block_on(build::run_build_with(
                                    renderer.as_ref(),
                                    &project,
                                    &script,
                                    &locale,
                                    &options,
                                    &mut show,
                                ))
                            }) {
                            Ok(report) => {
                                for w in &report.warnings {
                                    eprintln!("warning: {w}");
                                }
                                match cli.format {
                                    Format::Json => println!(
                                        "{}",
                                        serde_json::to_string_pretty(&report).unwrap()
                                    ),
                                    Format::Human => print!("{}", report.render()),
                                }
                                Outcome::Ok
                            }
                            Err(build::BuildError::Validation(errors)) => {
                                if cli.format == Format::Json {
                                    let report = ErrorReport::new(errors.clone());
                                    println!("{}", serde_json::to_string_pretty(&report).unwrap());
                                } else {
                                    for e in &errors {
                                        eprintln!("error: {e}");
                                    }
                                }
                                Outcome::ValidationError(errors)
                            }
                            Err(build::BuildError::Runtime(message)) => {
                                if cli.format == Format::Json {
                                    let report = ErrorReport::new(vec![message.clone()]);
                                    println!("{}", serde_json::to_string_pretty(&report).unwrap());
                                } else {
                                    eprintln!("error: {message}");
                                }
                                Outcome::RuntimeFailure(message)
                            }
                        }
                    }
                }
            }
        },
        Command::Capture {
            script,
            locale,
            resolution,
            fps,
        } => match Project::for_script(&script) {
            Err(e) => {
                eprintln!("error: {e}");
                Outcome::RuntimeFailure(e.to_string())
            }
            Ok(project) => {
                match resolution
                    .as_deref()
                    .map(build::parse_resolution)
                    .transpose()
                {
                    Err(e) => {
                        eprintln!("error: {e}");
                        Outcome::RuntimeFailure(e)
                    }
                    Ok(size) => {
                        let options = build::BuildOptions::defaults(&project, &script, &locale);
                        // Capture needs the manifest, and publishing it is
                        // `dub`'s job. Running it here rather than reading
                        // a stale one on disk is the same argument the
                        // renderer makes: two timing paths drift, and a
                        // clip captured against a length nothing published
                        // is a clip that does not fit.
                        let dubbed = runtime().map_err(dub::DubError::Runtime).and_then(|rt| {
                            rt.block_on(dub::run_dub(
                                &project,
                                &script,
                                &locale,
                                &options.narration_root,
                                false,
                            ))
                        });
                        match dubbed {
                            Err(e) => {
                                let message = match e {
                                    dub::DubError::Validation(d) => d.join("\n"),
                                    dub::DubError::Runtime(m) => m,
                                };
                                eprintln!("error: {message}");
                                Outcome::RuntimeFailure(message)
                            }
                            Ok(dubbed) => {
                                let (width, height) = size.unwrap_or(dubbed.output.resolution);
                                let frame = teleprompt_capture::Frame {
                                    width,
                                    height,
                                    fps: fps.unwrap_or(dubbed.output.fps),
                                };
                                let report = capture_cmd::run_capture(
                                    &dubbed.manifest,
                                    &dubbed.cues,
                                    &dubbed.scenes,
                                    &capture_cmd::registry(),
                                    &options.clips_dir,
                                    frame,
                                    &mut |p| {
                                        if cli.format == Format::Human {
                                            eprintln!(
                                                "  [{}/{}] {} {}",
                                                p.done, p.of, p.scene, p.cue
                                            );
                                        }
                                    },
                                );
                                for w in &report.warnings {
                                    eprintln!("warning: {w}");
                                }
                                match cli.format {
                                    Format::Json => println!(
                                        "{}",
                                        serde_json::to_string_pretty(&report).unwrap()
                                    ),
                                    Format::Human => print!("{}", report.render()),
                                }
                                Outcome::Ok
                            }
                        }
                    }
                }
            }
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
            Ok(project) => match runtime()
                .map_err(dub::DubError::Runtime)
                .and_then(|rt| rt.block_on(dub::run_dub(&project, &script, &locale, &out, check)))
            {
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
                        eprintln!("voice downgraded on {} line(s):", result.downgrades.len());
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
