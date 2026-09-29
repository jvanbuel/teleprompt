//! The models `teleprompt setup` installs, where it puts them: the app uses
//! them when Settings names none. As the CLI's `cmd::setup` finds them.

use std::path::PathBuf;

const SPEECH: &str = "sherpa-onnx-streaming-zipformer-en-2023-06-26";
const PUNCTUATION: &str = "sherpa-onnx-online-punct-en-2024-08-06";

/// `$TELEPROMPT_MODELS`, or `teleprompt/models` in the user's data
/// directory.
pub fn models_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("TELEPROMPT_MODELS") {
        return PathBuf::from(dir);
    }
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("teleprompt/models")
}

fn installed(name: &str) -> Option<PathBuf> {
    Some(models_dir().join(name)).filter(|p| p.is_dir())
}

/// The speech model `teleprompt setup speech-model` installed, if it did.
pub fn installed_speech() -> Option<PathBuf> {
    installed(SPEECH)
}

/// The punctuation model `teleprompt setup punctuation-model` installed.
pub fn installed_punctuation() -> Option<PathBuf> {
    installed(PUNCTUATION)
}
