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
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
            span: None,
            help: None,
        }
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
            span: None,
            help: None,
        }
    }

    pub fn at(mut self, span: SourceSpan) -> Self {
        self.span = Some(span);
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    pub fn render(&self, file: &str) -> String {
        let label = if self.is_error() { "error" } else { "warning" };
        let loc = match self.span {
            Some(s) => format!("{file}:{}:{}", s.line, s.column),
            None => file.to_string(),
        };
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
