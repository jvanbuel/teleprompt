//! `teleprompt setup`: the tools scene plugins run and the files backends read,
//! found on this machine or installed with the author's own package manager.
//! Teleprompt ships none of them (docs/design.md#what-teleprompt-ships): each
//! is under its own license, which `setup` says, and is the author's to
//! install.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;

mod catalogue;
pub mod plugins;

pub use catalogue::{download_mb, tools, Goal, GOALS};
use teleprompt_plugin::tool::Found;
pub use teleprompt_plugin::tool::{Manager, Tool};

/// The machine: its OS, the managers on it, and where models go.
#[derive(Debug, Clone)]
pub struct Platform {
    os: String,
    managers: Vec<Manager>,
    models: PathBuf,
}

impl Platform {
    pub fn new(os: &str, managers: &[Manager]) -> Self {
        Self {
            os: os.to_string(),
            managers: managers.to_vec(),
            models: models_dir(),
        }
    }

    /// This machine, as PATH shows it.
    pub fn detect() -> Self {
        let managers: Vec<Manager> = Manager::ALL
            .into_iter()
            .filter(|m| m.programs().iter().all(|p| on_path(p)))
            .collect();
        Self::new(std::env::consts::OS, &managers)
    }

    /// The command that installs `tool` here, or `None` when nothing on
    /// this machine can.
    pub fn command(&self, tool: &Tool) -> Option<String> {
        tool.command(self.preferred(), &self.models)
    }

    /// The system's own manager first, then Homebrew, then a language's.
    fn preferred(&self) -> impl Iterator<Item = Manager> + '_ {
        let system: &[Manager] = if self.os == "macos" {
            &[Manager::Brew]
        } else {
            &[Manager::Apt, Manager::Dnf, Manager::Pacman, Manager::Brew]
        };
        let rest = [
            Manager::Go,
            Manager::Cargo,
            Manager::Pipx,
            Manager::Npm,
            Manager::Download,
        ];
        system
            .iter()
            .copied()
            .chain(rest)
            .filter(|m| self.managers.contains(m))
    }
}

/// Where `setup` unpacks models, and where commands look for them:
/// `$TELEPROMPT_MODELS`, or `teleprompt/models` in the user's data directory.
pub fn models_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("TELEPROMPT_MODELS") {
        return PathBuf::from(dir);
    }
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    data.join("teleprompt/models")
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|d| d.join(program).is_file()))
}

/// Where `setup` put tool `name`'s model, if it is there.
fn installed_model(name: &str) -> Option<PathBuf> {
    let Found::Model(dir) = tools().into_iter().find(|t| t.name == name)?.found else {
        return None;
    };
    Some(models_dir().join(dir)).filter(|p| p.is_dir())
}

/// The speech model to listen with: `given`, or the one `setup` installed.
pub fn speech_model(given: Option<&Path>) -> Result<PathBuf, String> {
    given
        .map(Path::to_path_buf)
        .or_else(|| installed_model("speech-model"))
        .or_else(|| {
            crate::ask::offer(&["speech-model"], "listen")
                .then(|| installed_model("speech-model"))
                .flatten()
        })
        .ok_or_else(|| {
            format!(
                "no speech model: `teleprompt setup speech-model` installs one in {}, \
                 or --model names one",
                models_dir().display()
            )
        })
}

/// The speaker models `setup` installed, if any.
pub fn speaker_models() -> Option<PathBuf> {
    installed_model("speaker-model")
}

/// The punctuation model: `given`, or the one `setup` installed, if any.
pub fn punctuation_model(given: Option<&Path>) -> Option<PathBuf> {
    given
        .map(Path::to_path_buf)
        .or_else(|| installed_model("punctuation-model"))
}

/// The tools `names` stand for: a scene plugin's, or a tool by its own name;
/// each once, in order. Every tool when `names` is empty.
pub fn resolve(names: &[String]) -> Result<Vec<&'static Tool>, String> {
    if names.is_empty() {
        return Ok(tools());
    }
    let mut out: Vec<&'static Tool> = Vec::new();
    for name in names {
        let wanted: Vec<&str> = match GOALS.iter().find(|g| g.name == name) {
            Some(goal) => goal
                .needs
                .iter()
                .flat_map(|n| crate::scene::needs(n).unwrap_or_else(|| vec![*n]))
                .collect(),
            None => crate::scene::needs(name).unwrap_or_else(|| vec![name.as_str()]),
        };
        for w in wanted {
            let Some(tool) = tools().into_iter().find(|t| t.name == w) else {
                let plugins = crate::scene::plugins().names();
                let tools: Vec<&str> = tools().iter().map(|t| t.name).collect();
                return Err(format!(
                    "`{name}` is neither a scene plugin ({}) nor a tool ({})",
                    plugins.join(", "),
                    tools.join(", ")
                ));
            };
            if !out.iter().any(|t| t.name == tool.name) {
                out.push(tool);
            }
        }
    }
    Ok(out)
}

#[derive(Debug, Serialize)]
pub struct ToolStatus {
    pub name: &'static str,
    pub what: &'static str,
    /// `null` when `setup` cannot tell: the author's own project or server.
    pub installed: Option<bool>,
    pub license: &'static str,
    pub home: &'static str,
    /// What installs it here, or `null` when nothing on this machine can.
    pub command: Option<String>,
    pub guide: Option<&'static str>,
    /// How much it downloads, where `setup` knows: the models.
    pub download_mb: Option<u32>,
    /// Whether installing it asks for an administrator's password.
    pub password: bool,
}

/// One use of teleprompt, and what it needs here.
#[derive(Debug, Serialize)]
pub struct UseStatus {
    pub name: &'static str,
    pub label: &'static str,
    /// Whether it runs a speech model.
    pub listens: bool,
    /// Whether this teleprompt can do it at all: one that listens needs the
    /// build with speech models.
    pub available: bool,
    /// Whether everything it needs that `setup` can check is here.
    pub installed: bool,
    /// What its missing tools download, in megabytes.
    pub download_mb: u32,
    pub tools: Vec<ToolStatus>,
}

/// `setup --uses`: what teleprompt can be set up to do, as an app asks.
#[derive(Debug, Serialize)]
pub struct UsesReport {
    /// Whether this teleprompt was built with speech models.
    pub listening: bool,
    pub uses: Vec<UseStatus>,
    pub models: String,
}

#[derive(Debug, Serialize)]
pub struct SetupReport {
    pub tools: Vec<ToolStatus>,
    /// The commands run, with `--run`, in order.
    pub ran: Vec<String>,
    pub models: String,
    /// The project's own voice, in a project, when no tool was named.
    pub voice: Option<ProjectVoice>,
    /// The scene plugins installed as programs: every one when nothing was
    /// named, else those named.
    pub plugins: Vec<plugins::Installed>,
    /// Where plugins go when not on PATH.
    pub plugins_dir: String,
}

/// The voice the project chose, and whether it can speak here: the one
/// thing `setup` cannot see by looking for programs.
#[derive(Debug, Serialize)]
pub struct ProjectVoice {
    /// The project's `voice.backend`, `null` when it names none.
    pub backend: String,
    /// What its server said, or `null` when it has none to ask, as `null`
    /// has none. Not an error when it does not answer: only `dub` and
    /// `build` need it to (docs/design.md#backend-failure).
    pub answer: Option<String>,
    /// Every `backends:` setting that cannot be used, in the words `check`
    /// would use, including those for backends the project does not choose.
    pub problems: Vec<String>,
}

/// [`ProjectVoice`] for `project`, asking its backend's server.
pub async fn project_voice(project: &crate::project::Project) -> ProjectVoice {
    let backends = project.backends();
    let backend = project
        .config
        .voice
        .as_ref()
        .and_then(|v| v.backend.clone())
        .unwrap_or_else(|| "null".to_string());
    let problems = backends
        .diagnostics()
        .into_iter()
        .chain(backends.unusable_diagnostics())
        .map(|d| d.message)
        .collect();
    ProjectVoice {
        answer: backends.probe(&backend).await,
        backend,
        problems,
    }
}

/// Where `setup` looks and what it runs, fixed by the caller.
pub struct Setup {
    pub platform: Platform,
    /// Where npm installs and packages are looked for.
    pub project: PathBuf,
}

impl Setup {
    pub fn detect() -> Self {
        let here = PathBuf::from(".");
        let project = crate::project::Project::discover(&here)
            .map(|p| p.root.clone())
            .unwrap_or(here);
        Self {
            platform: Platform::detect(),
            project,
        }
    }

    pub fn report(&self, tools: &[&'static Tool], ran: Vec<String>) -> SetupReport {
        SetupReport {
            tools: tools.iter().map(|t| self.status(t)).collect(),
            ran,
            models: self.platform.models.display().to_string(),
            voice: None,
            plugins: Vec::new(),
            plugins_dir: teleprompt_plugin::protocol::host::plugins_dir()
                .display()
                .to_string(),
        }
    }

    fn status(&self, t: &'static Tool) -> ToolStatus {
        let command = self.platform.command(t);
        ToolStatus {
            name: t.name,
            what: t.what,
            installed: t.installed(&self.project, &self.platform.models),
            license: t.license,
            home: t.home,
            password: command.as_deref().is_some_and(needs_password),
            command,
            guide: t.guide,
            download_mb: download_mb(t.name),
        }
    }

    /// Every use, what it needs, and how much of that is here.
    pub fn uses(&self) -> UsesReport {
        let listening = cfg!(feature = "listen");
        let uses = GOALS
            .iter()
            .map(|goal| {
                let tools = resolve(&[goal.name.to_string()]).unwrap_or_default();
                let statuses: Vec<ToolStatus> = tools.iter().map(|t| self.status(t)).collect();
                let gone = statuses.iter().filter(|t| t.installed == Some(false));
                UseStatus {
                    name: goal.name,
                    label: goal.label,
                    listens: goal.listens,
                    available: listening || !goal.listens,
                    installed: statuses.iter().all(|t| t.installed != Some(false)),
                    download_mb: gone.filter_map(|t| t.download_mb).sum(),
                    tools: statuses,
                }
            })
            .collect();
        UsesReport {
            listening,
            uses,
            models: self.platform.models.display().to_string(),
        }
    }

    /// Whether `tool` is known to be missing here.
    pub fn missing(&self, tool: &Tool) -> bool {
        tool.installed(&self.project, &self.platform.models) == Some(false)
    }

    /// The command that would install `tool` here, if any.
    pub fn command(&self, tool: &Tool) -> Option<String> {
        self.platform.command(tool)
    }

    /// Runs the command for each tool that is missing, in order, stopping
    /// at the first that fails. What they print goes to stderr, so stdout
    /// stays the report; for an app (`--format json`), it is kept to say
    /// why one failed, and each tool's start, download and end are events.
    pub fn install(&self, tools: &[&'static Tool]) -> Result<Vec<String>, String> {
        let mut ran = Vec::new();
        for tool in tools {
            let missing = tool.installed(&self.project, &self.platform.models) == Some(false);
            let Some(command) = self.command(tool).filter(|_| missing) else {
                continue;
            };
            let command = without_a_terminal(&command)?;
            crate::output::progress(
                "install",
                || format!("installing {}: {command}", tool.name),
                serde_json::json!({ "tool": tool.name, "state": "start", "command": command }),
            );
            if crate::output::json_progress() {
                self.run_quietly(tool, &command)?;
            } else {
                self.run_aloud(&command)?;
            }
            crate::output::progress(
                "install",
                || format!("installed {}", tool.name),
                serde_json::json!({ "tool": tool.name, "state": "done" }),
            );
            ran.push(command);
        }
        Ok(ran)
    }

    /// Runs `command` where a person can see and answer it.
    fn run_aloud(&self, command: &str) -> Result<(), String> {
        let status = Command::new("/bin/sh")
            .arg("-c")
            .arg(command)
            .current_dir(&self.project)
            .stdin(Stdio::inherit())
            .stdout(to_stderr())
            .status()
            .map_err(|e| format!("`{command}` could not be run: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("`{command}` exited {status}"))
        }
    }

    /// Runs `command` for an app: its output kept in a log, which says why
    /// it failed if it does, and how far a model's download has got said
    /// as it goes.
    fn run_quietly(&self, tool: &Tool, command: &str) -> Result<(), String> {
        let log = std::env::temp_dir().join(format!("teleprompt-setup-{}.log", std::process::id()));
        let file = std::fs::File::create(&log).map_err(|e| format!("{}: {e}", log.display()))?;
        let err = file.try_clone().map_err(|e| e.to_string())?;
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg(command)
            .current_dir(&self.project)
            .stdin(Stdio::null())
            .stdout(file)
            .stderr(err)
            .spawn()
            .map_err(|e| format!("`{command}` could not be run: {e}"))?;
        let mut said = 0;
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                break status;
            }
            let of = download_mb(tool.name);
            // What is on disk, never more than the whole, whatever else a
            // download writes beside it.
            let mb = downloaded_mb(&self.platform.models).min(of.unwrap_or(0));
            if let (Some(of), true) = (of, mb > said) {
                said = mb;
                crate::output::progress(
                    "install",
                    String::new,
                    serde_json::json!({
                        "tool": tool.name, "state": "downloading", "mb": mb, "of": of,
                    }),
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        };
        let output = std::fs::read_to_string(&log).unwrap_or_default();
        let _ = std::fs::remove_file(&log);
        if status.success() {
            return Ok(());
        }
        let tail: Vec<&str> = output.lines().rev().take(5).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        Err(format!("`{command}` exited {status}:\n{}", tail.join("\n")))
    }
}

/// Whether `command` asks for an administrator's password.
fn needs_password(command: &str) -> bool {
    command.starts_with("sudo ") || command.contains(" sudo ")
}

/// `command` as it can run with nobody at a terminal to type a password:
/// as it is where it needs none, or where someone is there to type it;
/// through `pkexec`, the desktop's own password dialog, where there is
/// one; and otherwise not at all, saying to run it in a terminal.
fn without_a_terminal(command: &str) -> Result<String, String> {
    use std::io::IsTerminal;
    if !needs_password(command) || std::io::stdin().is_terminal() {
        return Ok(command.to_string());
    }
    if on_path("pkexec") {
        return Ok(command.replace("sudo ", "pkexec "));
    }
    Err(format!(
        "`{command}` asks for your password, and nothing here can ask for it: \
         run it in a terminal"
    ))
}

/// How much of a model's download is on disk so far: its `.download`
/// files in the models directory, in megabytes.
fn downloaded_mb(models: &Path) -> u32 {
    let bytes: u64 = std::fs::read_dir(models)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "download"))
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum();
    u32::try_from(bytes / 1_000_000).unwrap_or(u32::MAX)
}

fn to_stderr() -> Stdio {
    use std::os::fd::AsFd;
    std::io::stderr()
        .as_fd()
        .try_clone_to_owned()
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::inherit())
}

impl SetupReport {
    pub fn render(&self, names: &[String]) -> String {
        let mut out = String::new();
        for t in &self.tools {
            let state = match t.installed {
                Some(true) => "installed",
                Some(false) => "missing",
                None => "yours to set up",
            };
            out.push_str(&format!("{:<18} {state}: {}\n", t.name, t.what));
            if t.installed == Some(true) {
                continue;
            }
            out.push_str(&format!("{:<18} license  {}\n", "", t.license));
            match (&t.command, t.guide) {
                (_, Some(guide)) => out.push_str(&format!("{:<18} {guide}\n", "")),
                (Some(c), None) => out.push_str(&format!("{:<18} install  {c}\n", "")),
                (None, None) => out.push_str(&format!(
                    "{:<18} nothing here installs it; see {}\n",
                    "", t.home
                )),
            }
        }
        let dir = names.is_empty().then_some(self.plugins_dir.as_str());
        out.push_str(&plugins::render(&self.plugins, dir));
        if let Some(v) = &self.voice {
            let answer = v.answer.as_deref().unwrap_or("needs no server");
            out.push_str(&format!("{:<18} {}: {answer}\n", "your voice", v.backend));
            for p in &v.problems {
                out.push_str(&format!("{:<18} {p}\n", "problem"));
            }
        }
        for c in &self.ran {
            out.push_str(&format!("ran: {c}\n"));
        }
        let can_run = self
            .tools
            .iter()
            .any(|t| t.installed == Some(false) && t.command.is_some());
        if can_run && self.ran.is_empty() {
            let mut rerun = vec!["teleprompt", "setup"];
            rerun.extend(names.iter().map(String::as_str));
            rerun.push("--run");
            out.push_str(&format!(
                "\n`{}` runs these commands. Each tool is under its own license, \
                 and teleprompt ships none of them.\n",
                rerun.join(" ")
            ));
        }
        out
    }
}

impl UsesReport {
    pub fn render(&self) -> String {
        let width = self.uses.iter().map(|u| u.label.len()).max().unwrap_or(0) + 3;
        let mut out = String::new();
        for u in &self.uses {
            let state = if !u.available {
                "needs the build with speech models".to_string()
            } else if u.installed {
                "installed".to_string()
            } else {
                let gone: Vec<&str> = u
                    .tools
                    .iter()
                    .filter(|t| t.installed == Some(false))
                    .map(|t| t.name)
                    .collect();
                match u.download_mb {
                    0 => format!("needs {}", gone.join(", ")),
                    mb => format!("needs {} · {mb} MB", gone.join(", ")),
                }
            };
            out.push_str(&format!("{:<14}{:<width$}{state}\n", u.name, u.label));
        }
        out
    }
}

/// `setup`'s arguments.
#[derive(clap::Args)]
pub struct Args {
    pub names: Vec<String>,
    /// Run the commands that install what is missing
    #[arg(long)]
    pub run: bool,
    /// Say what teleprompt can be set up to do and what each use still
    /// needs, as the apps ask
    #[arg(long, conflicts_with_all = ["names", "run"])]
    pub uses: bool,
}

pub fn run(args: Args, format: crate::output::Format) -> crate::cli::Run {
    use crate::cli::{emit, runtime_failure};
    use crate::output::{Format, Outcome};
    let setup = Setup::detect();
    if args.uses {
        let report = setup.uses();
        emit(format, &report, &report.render());
        return Ok(Outcome::Ok);
    }
    // Asked rather than listed: what to do with teleprompt, not which tools.
    if args.names.is_empty() && !args.run && format == Format::Human && crate::ask::interactive() {
        let (chosen, tools) = crate::ask::choose(&setup).map_err(runtime_failure)?;
        let ran = crate::ask::confirm_install(&setup, &tools).map_err(runtime_failure)?;
        let mut report = setup.report(&tools, ran);
        report.voice = voice_here()?;
        report.plugins = plugins::installed();
        emit(format, &report, &report.render(&chosen));
        return Ok(Outcome::Ok);
    }
    let tools = resolve(&args.names).map_err(runtime_failure)?;
    let ran = if args.run {
        setup.install(&tools).map_err(runtime_failure)?
    } else {
        Vec::new()
    };
    let mut report = setup.report(&tools, ran);
    report.plugins = plugins::installed();
    if args.names.is_empty() {
        report.voice = voice_here()?;
    } else {
        report.plugins.retain(|p| args.names.contains(&p.name));
    }
    emit(format, &report, &report.render(&args.names));
    Ok(Outcome::Ok)
}

/// The voice of the project here, if this is one.
fn voice_here() -> Result<Option<ProjectVoice>, crate::output::Outcome> {
    let Ok(project) = crate::project::Project::discover(std::path::Path::new(".")) else {
        return Ok(None);
    };
    Ok(Some(
        crate::cli::runtime()?.block_on(project_voice(&project)),
    ))
}
