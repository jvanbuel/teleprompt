//! Where teleprompt keeps what belongs to the user rather than a project:
//! the XDG base directories on Linux and macOS, as command-line tools
//! keep them, and the known folders on Windows.

use std::path::PathBuf;

use etcetera::BaseStrategy;

/// `teleprompt/<what>` in the user's data directory (`$XDG_DATA_HOME`,
/// else `~/.local/share`; `%APPDATA%` on Windows), unless `$<env>` names
/// another directory.
pub fn data(env: &str, what: &str) -> PathBuf {
    if let Some(dir) = std::env::var_os(env).filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    etcetera::choose_base_strategy()
        .map(|base| base.data_dir())
        // With no home directory, the working directory stands in.
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("teleprompt")
        .join(what)
}
