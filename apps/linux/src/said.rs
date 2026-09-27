//! A line against what its take says, word by word: what keeping what was
//! said would drop, and what it would bring in.

/// A run of words, in reading order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Same(String),
    /// In the line, not said.
    Gone(String),
    /// Said, not in the line.
    New(String),
}

/// `line` against `said`, by their longest common run of words; in a gap,
/// what goes before what comes.
pub fn diff(line: &str, said: &str) -> Vec<Change> {
    let a: Vec<&str> = line.split_whitespace().collect();
    let b: Vec<&str> = said.split_whitespace().collect();
    let (n, m) = (a.len(), b.len());
    let mut common = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            common[i][j] = if a[i] == b[j] {
                common[i + 1][j + 1] + 1
            } else {
                common[i + 1][j].max(common[i][j + 1])
            };
        }
    }
    let mut out: Vec<Change> = Vec::new();
    let mut push = |change: Change| {
        let joined = match (out.last_mut(), &change) {
            (Some(Change::Same(run)), Change::Same(w))
            | (Some(Change::Gone(run)), Change::Gone(w))
            | (Some(Change::New(run)), Change::New(w)) => {
                run.push(' ');
                run.push_str(w);
                true
            }
            _ => false,
        };
        if !joined {
            out.push(change);
        }
    };
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            push(Change::Same(a[i].into()));
            (i, j) = (i + 1, j + 1);
        } else if i < n && (j == m || common[i + 1][j] >= common[i][j + 1]) {
            push(Change::Gone(a[i].into()));
            i += 1;
        } else {
            push(Change::New(b[j].into()));
            j += 1;
        }
    }
    out
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
