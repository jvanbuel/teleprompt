//! The app's settings: where `teleprompt` and the speech model are.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    pub binary: Option<PathBuf>,
    pub model: Option<PathBuf>,
    pub locale: Option<String>,
    pub last_script: Option<PathBuf>,
    /// Whether a take counts down from three before it listens.
    pub countdown: Option<bool>,
    /// A sherpa-onnx punctuation model, for a session's draft to have
    /// sentences. Optional.
    pub punctuation: Option<PathBuf>,
    /// The author's terminal, as it takes a command: `kitty`, `ghostty -e`.
    /// Found when unset.
    pub terminal: Option<String>,
}

impl Config {
    fn path() -> PathBuf {
        gtk::glib::user_config_dir().join("teleprompt/app.json")
    }

    /// The saved settings. `TELEPROMPT_BIN` and `TELEPROMPT_MODEL` override
    /// them for this run, and are not saved.
    pub fn load() -> Self {
        let mut config: Self = std::fs::read(Self::path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        if config.binary.is_none() {
            config.binary = default_binary();
        }
        config
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }

    pub fn binary(&self) -> Option<PathBuf> {
        std::env::var_os("TELEPROMPT_BIN")
            .map(PathBuf::from)
            .or_else(|| self.binary.clone())
    }

    pub fn model(&self) -> Option<PathBuf> {
        std::env::var_os("TELEPROMPT_MODEL")
            .map(PathBuf::from)
            .or_else(|| self.model.clone())
    }

    pub fn countdown(&self) -> bool {
        self.countdown.unwrap_or(true)
    }

    pub fn locale(&self) -> String {
        self.locale.clone().unwrap_or_else(|| "en".into())
    }
}

/// `teleprompt` on the PATH, or where `cargo install` puts it.
fn default_binary() -> Option<PathBuf> {
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|dir| dir.join("teleprompt"));
    let cargo = gtk::glib::home_dir().join(".cargo/bin/teleprompt");
    on_path.chain(std::iter::once(cargo)).find(|p| p.is_file())
}
