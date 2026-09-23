//! Drafting a teleprompt script from an existing Markdown document.

use std::collections::BTreeMap;

use crate::ast::slugify;

/// Languages whose fences become terminal tapes. Anything else is left as
/// ordinary Markdown: §3.2 ignores non-`teleprompt` fences for compilation,
/// so a JSON payload or a TypeScript snippet survives as an authoring note.
const SHELL: &[&str] = &["bash", "sh", "shell", "zsh", "console", "terminal"];

/// Turns a Markdown document into a teleprompt script.
///
/// `title` names the chapter that content before the document's own first
/// heading belongs to — §3.2 requires every line and action block to be
/// inside one, and a document that opens with prose has none. The caller
/// knows a name worth using (the file it read); this function would have to
/// invent one.
pub fn draft(markdown: &str, title: &str) -> String {
    let mut out = String::new();
    let mut paragraph: Vec<&str> = Vec::new();
    // Ids are the anchor for caching, translation and take binding, so two
    // of them colliding would make two lines one. Counted rather than
    // hashed: `run-the-build-2` still tells a reader which paragraph it is.
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut in_chapter = false;
    let mut lines = markdown.lines().peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim();

        if let Some(info) = trimmed.strip_prefix("```") {
            open_chapter(&mut out, &mut in_chapter, title);
            flush(&mut out, &mut paragraph, &mut seen);
            let mut body = Vec::new();
            for l in lines.by_ref() {
                if l.trim().starts_with("```") {
                    break;
                }
                body.push(l);
            }
            fence(&mut out, info.trim(), &body);
            continue;
        }

        if trimmed.is_empty() {
            flush(&mut out, &mut paragraph, &mut seen);
            continue;
        }

        if trimmed.starts_with('#') {
            flush(&mut out, &mut paragraph, &mut seen);
            in_chapter = true;
            out.push_str(trimmed);
            out.push_str("\n\n");
            continue;
        }

        open_chapter(&mut out, &mut in_chapter, title);
        paragraph.push(line.trim_end());
    }

    flush(&mut out, &mut paragraph, &mut seen);
    out
}

/// Opens the chapter that content before the document's first heading needs.
fn open_chapter(out: &mut String, in_chapter: &mut bool, title: &str) {
    if *in_chapter {
        return;
    }
    *in_chapter = true;
    out.push_str(&format!("# {title}\n\n"));
}

/// Emits the paragraph gathered so far as a line, with its id promoted.
fn flush(out: &mut String, paragraph: &mut Vec<&str>, seen: &mut BTreeMap<String, usize>) {
    if paragraph.is_empty() {
        return;
    }
    let text = paragraph.join("\n");
    paragraph.clear();
    let id = unique(id_for(&text), seen);
    out.push_str(&format!("{text} {{#{id}}}\n\n"));
}

/// `id`, or `id-2`, `id-3`… when the opening words have been used before.
fn unique(id: String, seen: &mut BTreeMap<String, usize>) -> String {
    let count = seen.entry(id.clone()).or_insert(0);
    *count += 1;
    if *count == 1 {
        id
    } else {
        format!("{id}-{count}")
    }
}

fn fence(out: &mut String, info: &str, body: &[&str]) {
    let language = info.split_whitespace().next().unwrap_or_default();
    if !SHELL.contains(&language) {
        out.push_str(&format!("```{info}\n{}\n```\n\n", body.join("\n")));
        return;
    }

    let mut tape = String::from("```teleprompt scene=terminal review=pending\n");
    for command in body.iter().map(|l| l.trim()).filter(|l| !l.is_empty()) {
        tape.push_str(&format!("Type \"{}\"\nEnter\nSleep 1s\n", escape(command)));
    }
    tape.push_str("```\n\n");
    out.push_str(&tape);
}

fn escape(command: &str) -> String {
    command.replace('\\', "\\\\").replace('"', "\\\"")
}

/// A line id derived from the paragraph's opening words.
fn id_for(paragraph: &str) -> String {
    let opening: Vec<&str> = paragraph.split_whitespace().take(3).collect();
    slugify(&opening.join(" "))
}

/// A draft script from a Slidev deck's speaker notes, and what was left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlidevDraft {
    pub script: String,
    /// 1-based numbers of slides with no notes, so nothing to say over
    /// them, and so not in the draft.
    pub silent: Vec<u32>,
}

/// Drafts a script from a Slidev deck: each slide's speaker notes become
/// its narration, split at Slidev's own `[click]` markers into one
/// paragraph per click step, each followed by a block showing that step.
///
/// `deck` is the path the scene config should name, as the build will
/// see it. The deck is read the way Slidev's own parser reads it: slides
/// are separated by `---` outside code fences and HTML comments, a
/// separator followed by YAML up to the next `---` is that slide's
/// frontmatter, and the notes are the slide's last comment, when nothing
/// comes after it. `[click]` adds one click and `[click:3]` adds three.
pub fn draft_slidev(deck_source: &str, deck: &str) -> SlidevDraft {
    let mut out = format!(
        "---\nteleprompt: 1\nscene:\n  slides:\n    adapter: slidev\n    deck: {deck}\n---\n\n"
    );
    let mut seen = BTreeMap::new();
    let mut silent = Vec::new();

    for (index, slide) in slides(deck_source).iter().enumerate() {
        let number = index as u32 + 1;
        let steps = note_steps(slide.note.as_deref().unwrap_or(""));
        if steps.is_empty() {
            silent.push(number);
            continue;
        }
        let title = slide
            .title
            .clone()
            .unwrap_or_else(|| format!("Slide {number}"));
        out.push_str(&format!("# {title}\n\n"));
        for (clicks, text) in steps {
            let id = unique(id_for(&text), &mut seen);
            let shot = if clicks == 0 {
                number.to_string()
            } else {
                format!("{number}?clicks={clicks}")
            };
            out.push_str(&format!(
                "{text} {{#{id}}}\n\n```teleprompt scene=slides policy=concurrent\n{shot}\n```\n\n"
            ));
        }
    }
    SlidevDraft {
        script: out,
        silent,
    }
}

/// One slide of a deck, as far as a draft needs it.
struct Slide {
    title: Option<String>,
    note: Option<String>,
}

/// The deck split into slides, by Slidev's rules.
fn slides(source: &str) -> Vec<Slide> {
    let lines: Vec<&str> = source.lines().collect();
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_comment = false;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim_end();
        if in_comment {
            in_comment = !line.contains("-->");
        } else if line.starts_with("---") {
            if start < i {
                out.push(slide(&lines[start..i]));
            }
            start = i + 1;
            let next = lines.get(i + 1).map_or("", |l| l.trim());
            if line.as_bytes().get(3) != Some(&b'-') && !next.is_empty() {
                // Frontmatter: this separator up to the next bare `---`
                // belongs to the slide it opens.
                start = i;
                i += 1;
                while i < lines.len() && lines[i].trim_end() != "---" {
                    i += 1;
                }
            }
        } else if line.trim_start().starts_with("```") {
            let fence: String = line
                .trim_start()
                .chars()
                .take_while(|c| *c == '`')
                .collect();
            if let Some(end) = (i + 1..lines.len()).find(|j| lines[*j].starts_with(&fence)) {
                i = end;
            }
        } else if let Some(at) = line.rfind("<!--") {
            in_comment = !line[at..].contains("-->");
        }
        i += 1;
    }
    if start < lines.len() {
        out.push(slide(&lines[start..]));
    }
    out
}

/// A slide's title and notes, from its lines.
fn slide(lines: &[&str]) -> Slide {
    let mut frontmatter_title = None;
    let mut body = lines;
    if lines
        .first()
        .is_some_and(|l| l.trim_end().starts_with("---"))
    {
        if let Some(end) = lines[1..].iter().position(|l| l.trim_end() == "---") {
            for line in &lines[1..=end] {
                if let Some(v) = line
                    .strip_prefix("title:")
                    .or_else(|| line.strip_prefix("name:"))
                {
                    frontmatter_title = Some(v.trim().trim_matches(['"', '\'']).to_string());
                }
            }
            body = &lines[end + 2..];
        }
    }
    let content = body.join("\n");
    let content = content.trim();

    let note = content
        .rfind("<!--")
        .filter(|at| {
            content.ends_with("-->") && !content[at + 4..content.len() - 3].contains("-->")
        })
        .map(|at| content[at + 4..content.len() - 3].trim().to_string());

    let title = frontmatter_title.filter(|t| !t.is_empty()).or_else(|| {
        let mut in_code = false;
        content.lines().find_map(|l| {
            if l.trim_start().starts_with("```") {
                in_code = !in_code;
            }
            if in_code {
                return None;
            }
            let hashes = l.chars().take_while(|c| *c == '#').count();
            (hashes > 0 && l[hashes..].starts_with(' ')).then(|| l[hashes..].trim().to_string())
        })
    });
    Slide { title, note }
}

/// Notes split at `[click]` markers into `(clicks, text)` steps, each
/// text one paragraph. Empty steps are left out; their clicks still count.
fn note_steps(note: &str) -> Vec<(u32, String)> {
    let mut steps = Vec::new();
    let mut clicks = 0;
    let mut rest = note;
    loop {
        let marker = find_click(rest);
        let text = marker.map_or(rest, |(at, _, _)| &rest[..at]);
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if !text.is_empty() {
            steps.push((clicks, text));
        }
        let Some((at, len, add)) = marker else {
            return steps;
        };
        clicks += add;
        rest = &rest[at + len..];
    }
}

/// The first `[click]` or `[click:N]` in `s`, case-insensitively, as
/// `(offset, length, clicks it adds)`.
fn find_click(s: &str) -> Option<(usize, usize, u32)> {
    let lower = s.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find("[click") {
        let at = from + at;
        let tail = &lower[at + 6..];
        if tail.starts_with(']') {
            return Some((at, 7, 1));
        }
        if let Some(n) = tail.strip_prefix(':') {
            let digits: String = n.chars().take_while(char::is_ascii_digit).collect();
            if !digits.is_empty() && n[digits.len()..].starts_with(']') {
                return Some((at, 6 + 1 + digits.len() + 1, digits.parse().unwrap_or(1)));
            }
        }
        from = at + 6;
    }
    None
}
