//! The one error every step returns.

/// Why a step failed: the script (exit 2), or anything else (exit 1).
#[derive(Debug)]
pub enum Failure {
    /// The script, or what was asked of it: each reason on its own.
    Validation(Vec<String>),
    /// Not the script's fault: a missing tool, a server, a file.
    Runtime(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation(reasons) => write!(f, "{}", reasons.join("\n")),
            Self::Runtime(message) => write!(f, "{message}"),
        }
    }
}
