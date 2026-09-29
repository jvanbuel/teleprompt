//! A line against what its take says, word by word, as the server works
//! it out: what keeping what was said would drop, and what it would bring
//! in.

/// A run of words, in reading order, as the server's script sends it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(tag = "kind", content = "words", rename_all = "lowercase")]
pub enum Change {
    Same(String),
    /// In the line, not said.
    Gone(String),
    /// Said, not in the line.
    New(String),
}

/// `changes` as Pango markup: what goes struck through, what comes bold.
pub fn markup(changes: &[Change]) -> String {
    let escape = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    changes
        .iter()
        .map(|c| match c {
            Change::Same(w) => escape(w),
            Change::Gone(w) => format!(
                "<span strikethrough=\"true\" foreground=\"#f28b82\">{}</span>",
                escape(w)
            ),
            Change::New(w) => format!(
                "<span weight=\"bold\" foreground=\"#81c995\">{}</span>",
                escape(w)
            ),
        })
        .collect::<Vec<_>>()
        .join(" ")
}
