//! `teleprompt setup`: the tools adapters run and the files backends read,
//! found on this machine or installed with the author's own package manager.
//! Teleprompt ships none of them (docs/design.md#what-teleprompt-ships): each
//! is under its own license, which `setup` says, and is the author's to
//! install.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;

/// A way to install software that may be on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manager {
    Brew,
    Apt,
    Dnf,
    Pacman,
    Go,
    Cargo,
    Pipx,
    /// npm, in the project: for tools a project depends on.
    Npm,
    /// `curl` and `tar`, for a download unpacked into the models directory.
    Download,
}

impl Manager {
    const ALL: [Manager; 9] = [
        Manager::Brew,
        Manager::Apt,
        Manager::Dnf,
        Manager::Pacman,
        Manager::Go,
        Manager::Cargo,
        Manager::Pipx,
        Manager::Npm,
        Manager::Download,
    ];

    /// The programs it needs on PATH.
    fn programs(self) -> &'static [&'static str] {
        match self {
            Manager::Brew => &["brew"],
            Manager::Apt => &["apt-get"],
            Manager::Dnf => &["dnf"],
            Manager::Pacman => &["pacman"],
            Manager::Go => &["go"],
            Manager::Cargo => &["cargo"],
            Manager::Pipx => &["pipx"],
            Manager::Npm => &["npm"],
            Manager::Download => &["curl", "tar"],
        }
    }
}

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

/// How to tell a tool is here.
#[derive(Debug, Clone, Copy)]
enum Found {
    /// A program that runs.
    Program(&'static str),
    /// An npm package in the project's `node_modules`.
    Package(&'static str),
    /// A directory in the models directory.
    Model(&'static str),
    /// Nothing `setup` can check: the author's own project or server.
    Unknowable,
}

/// Something an adapter or backend needs that teleprompt does not ship.
#[derive(Debug)]
pub struct Tool {
    pub name: &'static str,
    /// What teleprompt uses it for.
    pub what: &'static str,
    pub license: &'static str,
    pub home: &'static str,
    /// What to do instead, for what `setup` does not install.
    pub guide: Option<&'static str>,
    found: Found,
    install: &'static [(Manager, &'static str)],
}

impl Tool {
    /// The command that installs it here, or `None` when no way is.
    pub fn command(&self, platform: &Platform) -> Option<String> {
        platform.preferred().find_map(|m| {
            self.install
                .iter()
                .find(|(with, _)| *with == m)
                .map(|(_, c)| {
                    c.replace(
                        "{models}",
                        &shell_quote(&platform.models.display().to_string()),
                    )
                })
        })
    }

    /// Whether it is here, or `None` when `setup` cannot tell.
    pub fn installed(&self, project: &Path, models: &Path) -> Option<bool> {
        match self.found {
            Found::Program(p) => Some(teleprompt_capture::tool::installed(p)),
            Found::Package(p) => Some(project.join("node_modules").join(p).is_dir()),
            Found::Model(m) => Some(models.join(m).is_dir()),
            Found::Unknowable => None,
        }
    }
}

fn shell_quote(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-~".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

const SPEECH_MODEL: &str = "sherpa-onnx-streaming-zipformer-en-2023-06-26";
const PUNCTUATION_MODEL: &str = "sherpa-onnx-online-punct-en-2024-08-06";

static TOOLS: &[Tool] = &[
    Tool {
        name: "ffmpeg",
        what: "renders the video, and captures media, slide and terminal scenes",
        license: "LGPL-2.1-or-later, or GPL-2.0-or-later as many builds are",
        home: "https://ffmpeg.org",
        guide: None,
        found: Found::Program("ffmpeg"),
        install: &[
            (Manager::Brew, "brew install ffmpeg"),
            (Manager::Apt, "sudo apt-get install -y ffmpeg"),
            (Manager::Dnf, "sudo dnf install -y ffmpeg-free"),
            (Manager::Pacman, "sudo pacman -S --needed --noconfirm ffmpeg"),
        ],
    },
    Tool {
        name: "vhs",
        what: "records terminal tapes",
        license: "MIT",
        home: "https://github.com/charmbracelet/vhs",
        guide: None,
        found: Found::Program("vhs"),
        install: &[
            (Manager::Brew, "brew install vhs"),
            (Manager::Pacman, "sudo pacman -S --needed --noconfirm vhs"),
            (Manager::Go, "go install github.com/charmbracelet/vhs@latest"),
        ],
    },
    Tool {
        name: "ttyd",
        what: "the terminal vhs records",
        license: "MIT",
        home: "https://github.com/tsl0922/ttyd",
        guide: None,
        found: Found::Program("ttyd"),
        install: &[
            (Manager::Brew, "brew install ttyd"),
            (Manager::Apt, "sudo apt-get install -y ttyd"),
            (Manager::Dnf, "sudo dnf install -y ttyd"),
            (Manager::Pacman, "sudo pacman -S --needed --noconfirm ttyd"),
        ],
    },
    Tool {
        name: "asciinema",
        what: "records terminal sessions for `record`",
        license: "GPL-3.0",
        home: "https://asciinema.org",
        guide: None,
        found: Found::Program("asciinema"),
        install: &[
            (Manager::Brew, "brew install asciinema"),
            (Manager::Apt, "sudo apt-get install -y asciinema"),
            (Manager::Dnf, "sudo dnf install -y asciinema"),
            (Manager::Pacman, "sudo pacman -S --needed --noconfirm asciinema"),
            (Manager::Pipx, "pipx install asciinema"),
        ],
    },
    Tool {
        name: "agg",
        what: "draws asciinema casts as video",
        license: "Apache-2.0",
        home: "https://github.com/asciinema/agg",
        guide: None,
        found: Found::Program("agg"),
        install: &[
            (Manager::Brew, "brew install agg"),
            (
                Manager::Cargo,
                "cargo install --locked --git https://github.com/asciinema/agg",
            ),
        ],
    },
    Tool {
        name: "node",
        what: "runs Playwright, Remotion and Slidev",
        license: "MIT",
        home: "https://nodejs.org",
        guide: None,
        found: Found::Program("node"),
        install: &[
            (Manager::Brew, "brew install node"),
            (Manager::Apt, "sudo apt-get install -y nodejs npm"),
            (Manager::Dnf, "sudo dnf install -y nodejs npm"),
            (Manager::Pacman, "sudo pacman -S --needed --noconfirm nodejs npm"),
        ],
    },
    Tool {
        name: "playwright",
        what: "drives the browser for Playwright scenes, in this project",
        license: "Apache-2.0; the browsers it downloads have their own",
        home: "https://playwright.dev",
        guide: None,
        found: Found::Package("playwright"),
        install: &[(
            Manager::Npm,
            "npm install --save-dev playwright && npx playwright install chromium",
        )],
    },
    Tool {
        name: "slidev",
        what: "exports slides for Slidev scenes, in this project",
        license: "MIT",
        home: "https://sli.dev",
        guide: None,
        found: Found::Package("@slidev/cli"),
        install: &[(
            Manager::Npm,
            "npm install --save-dev @slidev/cli playwright-chromium",
        )],
    },
    Tool {
        name: "remotion",
        what: "renders compositions for Remotion scenes",
        license: "the Remotion License: free for individuals and small teams; \
                  a larger company needs a company license (https://www.remotion.dev/license)",
        home: "https://www.remotion.dev",
        guide: Some(
            "Remotion renders from a project of your own: run `npm install` in it, \
             and name it in the scene's `project` setting.",
        ),
        found: Found::Unknowable,
        install: &[],
    },
    Tool {
        name: "kokoro",
        what: "speaks the narration, when `voice.backend` is \"kokoro\"",
        license: "Apache-2.0, as are the Kokoro-82M weights",
        home: "https://github.com/remsky/Kokoro-FastAPI",
        guide: Some(
            "Kokoro is a server you run yourself, in Docker or with pip, \
             and `backends.kokoro.base_url` points at: see docs/guide/voices.md.",
        ),
        found: Found::Unknowable,
        install: &[],
    },
    Tool {
        name: "speech-model",
        what: "what `prompt`, `record` and `import` listen with",
        license: "Apache-2.0",
        home: "https://github.com/k2-fsa/sherpa-onnx",
        guide: None,
        found: Found::Model(SPEECH_MODEL),
        install: &[(
            Manager::Download,
            "mkdir -p {models} && curl -fL https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-streaming-zipformer-en-2023-06-26.tar.bz2 | tar xj -C {models}",
        )],
    },
    Tool {
        name: "punctuation-model",
        what: "what `record` and `import` punctuate drafts with",
        license: "Apache-2.0",
        home: "https://github.com/k2-fsa/sherpa-onnx",
        guide: None,
        found: Found::Model(PUNCTUATION_MODEL),
        install: &[(
            Manager::Download,
            "mkdir -p {models} && curl -fL https://github.com/k2-fsa/sherpa-onnx/releases/download/punctuation-models/sherpa-onnx-online-punct-en-2024-08-06.tar.bz2 | tar xj -C {models}",
        )],
    },
];

/// What each adapter runs, as `setup <adapter>` installs it.
const ADAPTERS: &[(&str, &[&str])] = &[
    ("vhs", &["vhs", "ttyd", "ffmpeg"]),
    ("asciinema", &["asciinema", "agg", "ffmpeg"]),
    ("playwright", &["node", "ffmpeg", "playwright"]),
    ("remotion", &["node", "remotion"]),
    ("slidev", &["node", "ffmpeg", "slidev"]),
    ("media", &["ffmpeg"]),
    ("mock", &["ffmpeg"]),
    ("prompt", &["speech-model"]),
];

pub fn tools() -> &'static [Tool] {
    TOOLS
}

/// Where `setup` put tool `name`'s model, if it is there.
fn installed_model(name: &str) -> Option<PathBuf> {
    let Found::Model(dir) = TOOLS.iter().find(|t| t.name == name)?.found else {
        return None;
    };
    Some(models_dir().join(dir)).filter(|p| p.is_dir())
}

/// The speech model to listen with: `given`, or the one `setup` installed.
pub fn speech_model(given: Option<&Path>) -> Result<PathBuf, String> {
    given
        .map(Path::to_path_buf)
        .or_else(|| installed_model("speech-model"))
        .ok_or_else(|| {
            format!(
                "no speech model: `teleprompt setup speech-model` installs one in {}, \
                 or --model names one",
                models_dir().display()
            )
        })
}

/// The punctuation model: `given`, or the one `setup` installed, if any.
pub fn punctuation_model(given: Option<&Path>) -> Option<PathBuf> {
    given
        .map(Path::to_path_buf)
        .or_else(|| installed_model("punctuation-model"))
}

/// The tools `names` stand for: an adapter's, or a tool by its own name;
/// each once, in order. Every tool when `names` is empty.
pub fn resolve(names: &[String]) -> Result<Vec<&'static Tool>, String> {
    if names.is_empty() {
        return Ok(TOOLS.iter().collect());
    }
    let mut out: Vec<&'static Tool> = Vec::new();
    for name in names {
        let wanted: Vec<&str> = match ADAPTERS.iter().find(|(a, _)| a == name) {
            Some((_, tools)) => tools.to_vec(),
            None => vec![name.as_str()],
        };
        for w in wanted {
            let Some(tool) = TOOLS.iter().find(|t| t.name == w) else {
                let adapters: Vec<&str> = ADAPTERS.iter().map(|(a, _)| *a).collect();
                let tools: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
                return Err(format!(
                    "`{name}` is neither an adapter ({}) nor a tool ({})",
                    adapters.join(", "),
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
}

#[derive(Debug, Serialize)]
pub struct SetupReport {
    pub tools: Vec<ToolStatus>,
    /// The commands run, with `--run`, in order.
    pub ran: Vec<String>,
    pub models: String,
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
            tools: tools
                .iter()
                .map(|t| ToolStatus {
                    name: t.name,
                    what: t.what,
                    installed: t.installed(&self.project, &self.platform.models),
                    license: t.license,
                    home: t.home,
                    command: t.command(&self.platform),
                    guide: t.guide,
                })
                .collect(),
            ran,
            models: self.platform.models.display().to_string(),
        }
    }

    /// Runs the command for each tool that is missing, in order, stopping
    /// at the first that fails. What they print goes to stderr, so stdout
    /// stays the report.
    pub fn install(&self, tools: &[&'static Tool]) -> Result<Vec<String>, String> {
        let mut ran = Vec::new();
        for tool in tools {
            let missing = tool.installed(&self.project, &self.platform.models) == Some(false);
            let Some(command) = tool.command(&self.platform).filter(|_| missing) else {
                continue;
            };
            eprintln!("installing {}: {command}", tool.name);
            let status = Command::new("/bin/sh")
                .arg("-c")
                .arg(&command)
                .current_dir(&self.project)
                .stdin(Stdio::inherit())
                .stdout(to_stderr())
                .status()
                .map_err(|e| format!("`{command}` could not be run: {e}"))?;
            if !status.success() {
                return Err(format!("`{command}` exited {status}"));
            }
            ran.push(command);
        }
        Ok(ran)
    }
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
