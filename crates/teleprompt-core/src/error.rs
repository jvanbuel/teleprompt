#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSpan {
    pub line: usize,
    pub column: usize,
    pub len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub span: Option<SourceSpan>,
    /// The file this diagnostic is about, when that is *not* the script the
    /// caller is rendering. `None` — the common case — means the script.
    /// Set for content the script pulls in from elsewhere, such as an action
    /// block body loaded with `include=`: pointing a line number from a
    /// separate file at the script's own path names a line that may not even
    /// exist there.
    pub file: Option<String>,
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
            span: None,
            file: None,
            help: None,
        }
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
            span: None,
            file: None,
            help: None,
        }
    }

    #[must_use]
    pub fn at(mut self, span: SourceSpan) -> Self {
        self.span = Some(span);
        self
    }

    /// Point this diagnostic at a file other than the script being rendered.
    #[must_use]
    pub fn in_file(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }

    #[must_use]
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    /// As a person reads it, pointing at `file` unless it names its own.
    /// With neither a file nor a span there is nothing to point at, so it
    /// is the message alone, for whoever prints it to label.
    pub fn render(&self, file: &str) -> String {
        let label = if self.is_error() { "error" } else { "warning" };
        let file = self.file.as_deref().unwrap_or(file);
        let loc = match self.span {
            Some(s) => format!("{file}:{}:{}", s.line, s.column),
            None => file.to_string(),
        };
        if loc.is_empty() {
            return self.message.clone();
        }
        let mut out = format!("{label}: {}\n  --> {loc}", self.message);
        if let Some(h) = &self.help {
            out.push_str(&format!("\n  help: {h}"));
        }
        out
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{} problem(s) found", .0.len())]
pub struct Diagnostics(pub Vec<Diagnostic>);

impl Diagnostics {
    pub fn has_errors(&self) -> bool {
        self.0.iter().any(Diagnostic::is_error)
    }

    /// One error, with no place it points at.
    pub fn error(message: impl Into<String>) -> Self {
        Self(vec![Diagnostic::error(message)])
    }

    /// Each as a person reads it, pointing where it says it is about.
    pub fn render(&self) -> Vec<String> {
        self.0.iter().map(|d| d.render("")).collect()
    }

    /// Each pointed at `file`, where it does not name its own: what a
    /// script's problems are given once the script is known, so they can
    /// be read anywhere.
    #[must_use]
    pub fn about(mut self, file: &str) -> Self {
        for d in &mut self.0 {
            d.file.get_or_insert_with(|| file.to_string());
        }
        self
    }
}

impl From<Vec<Diagnostic>> for Diagnostics {
    fn from(diagnostics: Vec<Diagnostic>) -> Self {
        Self(diagnostics)
    }
}

/// `e` and what caused it, each once: `a: b: c`. An HTTP client's error
/// says only which request failed; why (refused, no such host, a bad
/// certificate) is in its sources.
pub fn with_causes(e: &(dyn std::error::Error + 'static)) -> String {
    let mut out = e.to_string();
    let mut next = e.source();
    while let Some(cause) = next {
        let said = cause.to_string();
        if !out.contains(&said) {
            out.push_str(": ");
            out.push_str(&said);
        }
        next = cause.source();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_renders_file_line_and_column() {
        let d = Diagnostic::error("unknown attribute key `polcy`")
            .at(SourceSpan {
                line: 12,
                column: 5,
                len: 5,
            })
            .with_help("did you mean `policy`?");
        let rendered = d.render("scripts/demo.md");
        assert!(rendered.contains("scripts/demo.md:12:5"));
        assert!(rendered.contains("unknown attribute key `polcy`"));
        assert!(rendered.contains("did you mean `policy`?"));
    }

    #[test]
    fn warning_severity_is_not_error() {
        assert!(!Diagnostic::warning("bare wait").is_error());
    }
}
