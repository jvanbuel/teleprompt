//! The one error every step returns.

use teleprompt_core::Diagnostics;

/// Why a step failed: the script (exit 2), or anything else (exit 1).
#[derive(Debug)]
pub enum Failure {
    /// The script, or what was asked of it: each problem, and where.
    Validation(Diagnostics),
    /// Not the script's fault: a missing tool, a server, a file.
    Runtime(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation(problems) => write!(f, "{}", problems.render().join("\n")),
            Self::Runtime(message) => write!(f, "{message}"),
        }
    }
}
