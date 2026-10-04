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
            "mkdir -p {models} && curl -fL -o {models}/speech-model.download https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-streaming-zipformer-en-2023-06-26.tar.bz2 && tar xjf {models}/speech-model.download -C {models} && rm {models}/speech-model.download",
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
            "mkdir -p {models} && curl -fL -o {models}/punctuation-model.download https://github.com/k2-fsa/sherpa-onnx/releases/download/punctuation-models/sherpa-onnx-online-punct-en-2024-08-06.tar.bz2 && tar xjf {models}/punctuation-model.download -C {models} && rm {models}/punctuation-model.download",
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
            "rm -rf {models}/speaker-models.partial && mkdir -p {models}/speaker-models.partial && curl -fL -o {models}/speaker-segmentation.download https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2 && curl -fL -o {models}/speaker-embedding.download https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx && tar xjf {models}/speaker-segmentation.download -C {models}/speaker-models.partial && mv {models}/speaker-embedding.download {models}/speaker-models.partial/embedding.onnx && rm {models}/speaker-segmentation.download && mv {models}/speaker-models.partial {models}/speaker-models",
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
                let adapters = crate::scene::adapter_names();
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
    let backends = crate::cmd::check::backends_of(project);
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
        }
    }

    fn status(&self, t: &'static Tool) -> ToolStatus {
        let command = t.command(&self.platform);
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
        tool.command(&self.platform)
    }

    /// Runs the command for each tool that is missing, in order, stopping
    /// at the first that fails. What they print goes to stderr, so stdout
    /// stays the report; for an app (`--format json`), it is kept to say
    /// why one failed, and each tool's start, download and end are events.
    pub fn install(&self, tools: &[&'static Tool]) -> Result<Vec<String>, String> {
        let mut ran = Vec::new();
        for tool in tools {
            let missing = tool.installed(&self.project, &self.platform.models) == Some(false);
            let Some(command) = tool.command(&self.platform).filter(|_| missing) else {
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
