//! What `setup` knows how to find and install: what each plugin says it
//! needs, the speech models the CLI itself listens with, and the uses of
//! teleprompt that ask for them. A plugin's tools are the plugin's own
//! (`teleprompt_plugin::tool::Tool`); adding one is an entry there.

use teleprompt_plugin::tool::{Found, Manager, Tool};

const SPEECH_MODEL: &str = "sherpa-onnx-streaming-zipformer-en-2023-06-26";
const PUNCTUATION_MODEL: &str = "sherpa-onnx-online-punct-en-2024-08-06";
const SPEAKER_MODELS: &str = "speaker-models";

/// The models the CLI listens with: not a plugin's, since listening is
/// teleprompt's own.
static MODELS: &[Tool] = &[
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
        download_mb: Some(310),
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
        download_mb: Some(31),
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
        download_mb: Some(47),
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
    /// Scene plugins and tools, as `resolve` takes them.
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

/// Every tool `setup` knows, each once: what renders the video, what each
/// plugin needs, in the order the CLI registers them, and the models.
pub fn tools() -> Vec<&'static Tool> {
    let mut out: Vec<&'static Tool> = vec![&teleprompt_plugin::tool::FFMPEG];
    let plugins = crate::scene::plugin_needs()
        .into_iter()
        .chain(crate::voice::needs());
    for tool in plugins.chain(MODELS.iter()) {
        if !out.iter().any(|t| t.name == tool.name) {
            out.push(tool);
        }
    }
    out
}

/// How much tool `name` downloads, for saying so before it does.
pub fn download_mb(name: &str) -> Option<u32> {
    tools().into_iter().find(|t| t.name == name)?.download_mb
}
