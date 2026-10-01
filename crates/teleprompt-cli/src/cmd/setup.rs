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
const SPEAKER_MODELS: &str = "speaker-models";

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
        name: "Xvfb",
        what: "the virtual display an x11 scene's app runs on",
        license: "MIT",
        home: "https://www.x.org",
        guide: None,
        found: Found::Program("Xvfb"),
        install: &[
            (Manager::Apt, "sudo apt-get install -y xvfb"),
            (Manager::Dnf, "sudo dnf install -y xorg-x11-server-Xvfb"),
            (Manager::Pacman, "sudo pacman -S --needed --noconfirm xorg-server-xvfb"),
        ],
    },
    Tool {
        name: "xdotool",
        what: "presses keys and moves the pointer in an x11 scene",
        license: "BSD-3-Clause",
        home: "https://github.com/jordansissel/xdotool",
        guide: None,
        found: Found::Program("xdotool"),
        install: &[
            (Manager::Apt, "sudo apt-get install -y xdotool"),
            (Manager::Dnf, "sudo dnf install -y xdotool"),
            (Manager::Pacman, "sudo pacman -S --needed --noconfirm xdotool"),
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
        name: "gemini",
        what: "speaks the narration, when `voice.backend` is \"gemini\": Google's Gemini 3.8 TTS",
        license: "a hosted service under Google's Gemini API terms \
                  (https://ai.google.dev/gemini-api/terms); its audio carries a SynthID watermark",
        home: "https://ai.google.dev/gemini-api/docs/speech-generation",
        guide: Some(
            "Gemini runs at Google, not here, and the narration is sent there: \
             put an API key from aistudio.google.com/apikey in GEMINI_API_KEY. \
             See docs/guide/voices.md.",
        ),
        found: Found::Unknowable,
        install: &[],
    },
    Tool {
        name: "voicebox",
        what: "speaks the narration in a cloned or designed voice, when `voice.backend` is \"voicebox\"",
        license: "MIT, the app; its models have their own: Qwen3-TTS, LuxTTS and Kokoro Apache-2.0, \
                  Chatterbox MIT (its audio is watermarked), and TADA's weights the Llama 3.2 \
                  Community License, which has conditions",
        home: "https://voicebox.sh",
        guide: Some(
            "Voicebox is an app you run yourself (from voicebox.sh, or in Docker); set \
             `voice.backend = \"voicebox\"` and make a voice from your takes with \
             `teleprompt voice clone <name>`: see docs/guide/voices.md.",
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
    Tool {
        name: "speaker-model",
        what: "what `from` tells the voices in a recording apart with",
        license: "MIT (pyannote segmentation), Apache-2.0 (3D-Speaker embedding)",
        home: "https://github.com/k2-fsa/sherpa-onnx",
        guide: None,
        found: Found::Model(SPEAKER_MODELS),
        // Into a scratch directory first, renamed into place once both are
        // there, so a failed download never looks installed.
        install: &[(
            Manager::Download,
            "rm -rf {models}/speaker-models.partial && mkdir -p {models}/speaker-models.partial && curl -fL https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2 | tar xj -C {models}/speaker-models.partial && curl -fL -o {models}/speaker-models.partial/embedding.onnx https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx && mv {models}/speaker-models.partial {models}/speaker-models",
        )],
    },
];

/// Something to do with teleprompt, and what it needs: how `setup` asks,
/// since nobody wants a segmentation model, but some want to turn an
/// interview into a video.
#[derive(Debug)]
pub struct Goal {
    /// What `setup` takes as a name for it.
    pub name: &'static str,
    pub label: &'static str,
    /// Adapters and tools, as `resolve` takes them.
    pub needs: &'static [&'static str],
    /// Whether it runs a speech model, which only the build with them can.
    pub listens: bool,
}

pub static GOALS: &[Goal] = &[
    Goal {
        name: "render",
        label: "Render videos",
        needs: &["ffmpeg"],
        listens: false,
    },
    Goal {
        name: "terminal",
        label: "Show a terminal, typed as it runs (VHS)",
        needs: &["vhs"],
        listens: false,
    },
    Goal {
        name: "casts",
        label: "Show terminal recordings (asciinema)",
        needs: &["asciinema"],
        listens: false,
    },
    Goal {
        name: "browser",
        label: "Show a browser, scripted (Playwright)",
        needs: &["playwright"],
        listens: false,
    },
    Goal {
        name: "slides",
        label: "Show slides (Slidev)",
        needs: &["slidev"],
        listens: false,
    },
    Goal {
        name: "desktop",
        label: "Show a desktop app",
        needs: if cfg!(target_os = "macos") {
            &["macos"]
        } else {
            &["x11"]
        },
        listens: false,
    },
    Goal {
        name: "prompt",
        label: "Have the prompter follow your voice",
        needs: &["speech-model"],
        listens: true,
    },
    Goal {
        name: "drafts",
        label: "Draft scripts from what you said (record, import)",
        needs: &["speech-model", "punctuation-model"],
        listens: true,
    },
    Goal {
        name: "conversations",
        label: "Turn a recorded conversation into a script",
        needs: &["speech-model", "punctuation-model", "speaker-model"],
        listens: true,
    },
];

/// How much a model downloads, for saying so before it does.
pub fn download_mb(tool: &str) -> Option<u32> {
    match tool {
        "speech-model" => Some(310),
        "punctuation-model" => Some(31),
        "speaker-model" => Some(47),
        _ => None,
    }
}

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

/// The tools `names` stand for: an adapter's, or a tool by its own name;
/// each once, in order. Every tool when `names` is empty.
pub fn resolve(names: &[String]) -> Result<Vec<&'static Tool>, String> {
    if names.is_empty() {
        return Ok(TOOLS.iter().collect());
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
            let Some(tool) = TOOLS.iter().find(|t| t.name == w) else {
                let captures = crate::scene::captures();
                let adapters: Vec<&str> = captures.backends().map(|b| b.adapter()).collect();
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

    /// Whether `tool` is known to be missing here.
    pub fn missing(&self, tool: &Tool) -> bool {
        tool.installed(&self.project, &self.platform.models) == Some(false)
    }

    /// The command that would install `tool` here, if any.
    pub fn command(&self, tool: &Tool) -> Option<String> {
        tool.command(&self.platform)
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
