//! Drafting a teleprompt script from an existing Markdown document.

use std::collections::BTreeMap;

use teleprompt_core::ast::slugify;

/// Languages whose fences become terminal tapes. Anything else is left as
/// ordinary Markdown: compilation ignores non-`teleprompt` fences, so a JSON
/// payload or a TypeScript snippet survives as an authoring note.
const SHELL: &[&str] = &["bash", "sh", "shell", "zsh", "console", "terminal"];

/// Turns a Markdown document into a teleprompt script.
///
/// `title` names the chapter that content before the document's own first
/// heading belongs to — every line and action block must be inside one, and a
/// document that opens with prose has none. The caller knows a name worth using
/// (the file it read); this function would have to invent one.
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
pub(crate) fn unique(id: String, seen: &mut BTreeMap<String, usize>) -> String {
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

    let mut tape = String::from("```teleprompt scene=vhs review=pending\n");
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
pub(crate) fn id_for(paragraph: &str) -> String {
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
    /// What else the draft could not follow — a hidden slide's notes, an
    /// import that could not be read — one sentence each.
    pub warnings: Vec<String>,
}

/// Drafts a script from a Slidev deck: each slide's speaker notes become
/// its narration, split at Slidev's `[click]` markers (`[click:3]` adds
/// three) into one paragraph per click step, each followed by a block
/// showing that step.
///
/// `deck` is the path the scene config should name, as the build will see
/// it. Slides are read and numbered as Slidev does: a hidden or disabled
/// slide has no number, and a `src:` import contributes the slides of the
/// file it names. `read` returns an import's contents given its path
/// relative to the deck's directory, which keeps this function free of IO.
pub fn draft_slidev(
    deck_source: &str,
    deck: &str,
    read: &dyn Fn(&str) -> Option<String>,
) -> SlidevDraft {
    let mut loaded = Vec::new();
    let mut warnings = Vec::new();
    load(deck_source, "", None, &[], read, &mut loaded, &mut warnings);

    let mut out = format!("---\nteleprompt: 1\nscene:\n  slidev:\n    deck: {deck}\n---\n\n");
    let mut seen = BTreeMap::new();
    let mut silent = Vec::new();
    for (index, slide) in loaded.iter().enumerate() {
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
                "{text} {{#{id}}}\n\n```teleprompt scene=slidev policy=concurrent\n{shot}\n```\n\n"
            ));
        }
    }
    SlidevDraft {
        script: out,
        silent,
        warnings,
    }
}

/// The slides of one file, in the order Slidev shows them, appended to
/// `into`. `file` is its path relative to the deck's directory (empty for
/// the deck itself), `range` the 1-based slides an import selected, and
/// `chain` the files importing it, for refusing a cycle.
fn load(
    source: &str,
    file: &str,
    range: Option<&[usize]>,
    chain: &[String],
    read: &dyn Fn(&str) -> Option<String>,
    into: &mut Vec<Slide>,
    warnings: &mut Vec<String>,
) {
    let named = |slide: &Slide, n: usize| {
        slide.title.clone().map_or_else(
            || format!("slide {n} of {}", or_deck(file)),
            |t| format!("`{t}`"),
        )
    };
    for (i, slide) in slides(source).into_iter().enumerate() {
        let n = i + 1;
        if range.is_some_and(|r| !r.contains(&n)) {
            continue;
        }
        if slide.hidden {
            if slide.note.is_some() {
                warnings.push(format!(
                    "{} is hidden, so it has no slide number, and its notes are not in the draft",
                    named(&slide, n)
                ));
            }
            continue;
        }
        let Some(src) = &slide.src else {
            into.push(slide);
            continue;
        };
        let (raw, selection) = src.split_once('#').unwrap_or((src.as_str(), ""));
        let path = resolve(file, raw);
        if path == file || chain.contains(&path) {
            warnings.push(format!(
                "`src: {src}` in {} imports itself; left out",
                or_deck(file)
            ));
            continue;
        }
        let Some(imported) = read(&path) else {
            warnings.push(format!(
                "`src: {src}` in {} could not be read, so every slide number after it may be wrong",
                or_deck(file)
            ));
            continue;
        };
        let selection = (!selection.is_empty()).then(|| parse_range(selection));
        let mut chain = chain.to_vec();
        chain.push(file.to_string());
        load(
            &imported,
            &path,
            selection.as_deref(),
            &chain,
            read,
            into,
            warnings,
        );
    }
}

fn or_deck(file: &str) -> &str {
    if file.is_empty() {
        "the deck"
    } else {
        file
    }
}

/// An import's path relative to the deck's directory: `/x.md` is rooted
/// there, anything else is relative to the importing file.
fn resolve(importer: &str, raw: &str) -> String {
    let joined = match raw.strip_prefix('/') {
        Some(rooted) => rooted.to_string(),
        None => match importer.rsplit_once('/') {
            Some((dir, _)) => format!("{dir}/{raw}"),
            None => raw.to_string(),
        },
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

/// Slidev's range syntax, `2`, `1-3`, `1,4-5`, as 1-based slide numbers.
fn parse_range(s: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for part in s.split(',') {
        match part.split_once('-') {
            Some((a, b)) => {
                if let (Ok(a), Ok(b)) = (a.trim().parse::<usize>(), b.trim().parse::<usize>()) {
                    out.extend(a..=b);
                }
            }
            None => out.extend(part.trim().parse::<usize>()),
        }
    }
    out
}

/// One slide of a deck, as far as a draft needs it.
struct Slide {
    title: Option<String>,
    note: Option<String>,
    /// `hide: true` or `disabled: true`: Slidev drops it, number and all.
    hidden: bool,
    /// `src:` — the slide is the file it names.
    src: Option<String>,
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

/// A slide's title, notes and the frontmatter that decides its number.
fn slide(lines: &[&str]) -> Slide {
    let mut frontmatter: BTreeMap<&str, String> = BTreeMap::new();
    let mut body = lines;
    if lines
        .first()
        .is_some_and(|l| l.trim_end().starts_with("---"))
    {
        if let Some(end) = lines[1..].iter().position(|l| l.trim_end() == "---") {
            for line in &lines[1..=end] {
                if let Some((key, value)) = line.split_once(':') {
                    if !key.starts_with(char::is_whitespace) {
                        let value = value.trim().trim_matches(['"', '\'']).to_string();
                        frontmatter.insert(key.trim(), value);
                    }
                }
            }
            body = &lines[end + 2..];
        }
    }
    let content = body.join("\n");
    let content = content.trim();

    // `get`, not indexing: in `<!-->` the comment's ends overlap.
    let note = content
        .rfind("<!--")
        .filter(|_| content.ends_with("-->"))
        .and_then(|at| content.get(at + 4..content.len() - 3))
        .filter(|inner| !inner.contains("-->"))
        .map(|inner| inner.trim().to_string());

    let title = frontmatter
        .get("title")
        .or_else(|| frontmatter.get("name"))
        .filter(|t| !t.is_empty())
        .cloned()
        .or_else(|| {
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
    let flag = |key: &str| frontmatter.get(key).is_some_and(|v| v == "true");
    Slide {
        title,
        note,
        hidden: flag("hide") || flag("disabled"),
        src: frontmatter.get("src").filter(|s| !s.is_empty()).cloned(),
    }
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
