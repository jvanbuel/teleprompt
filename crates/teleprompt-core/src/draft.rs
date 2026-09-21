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
/// heading belongs to — §3.2 requires every segment and action block to be
/// inside one, and a document that opens with prose has none. The caller
/// knows a name worth using (the file it read); this function would have to
/// invent one.
pub fn draft(markdown: &str, title: &str) -> String {
    let mut out = String::new();
    let mut paragraph: Vec<&str> = Vec::new();
    // Ids are the anchor for caching, translation and take binding, so two
    // of them colliding would make two segments one. Counted rather than
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

/// Emits the paragraph gathered so far as a segment, with its id promoted.
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

/// A segment id derived from the paragraph's opening words.
fn id_for(paragraph: &str) -> String {
    let opening: Vec<&str> = paragraph.split_whitespace().take(3).collect();
    slugify(&opening.join(" "))
}
