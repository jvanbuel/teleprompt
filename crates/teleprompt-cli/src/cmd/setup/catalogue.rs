//! What `setup` knows how to find and install: each tool and model an
//! adapter or backend needs, with its license and the command per package
//! manager, and the uses of teleprompt that ask for them. Data, mostly;
//! adding a tool is an entry here.

use std::path::Path;

use super::{Manager, Platform};

/// How to tell a tool is here.
#[derive(Debug, Clone, Copy)]
pub(super) enum Found {
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
    pub(super) found: Found,
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
            Found::Program(p) => Some(teleprompt_plugin::tool::installed(p)),
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

pub(super) static TOOLS: &[Tool] = &[
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
        what: "what `serve`, `record` and `import` listen with",
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
        what: "what `import` tells the voices in a recording apart with",
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
