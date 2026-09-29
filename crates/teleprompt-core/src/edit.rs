//! Editing a script where a timeline drag says: a block's start moved
//! along its line, off it, or onto another line, or its shots stretched;
//! or a line reworded to what its take says.
//!
//! Every edit is to the text, the fence's attributes or a block's place,
//! and leaves the rest of the file as the author wrote it. A drag never
//! touches a line: narration is never re-timed, only the shots are.

use std::ops::Range;

use crate::ast::Node;
use crate::parse::parse_script;

/// One edit, naming blocks and lines by their ids as `plan` shows them.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    /// Run the block with its line, starting at the line's `word`th word
    /// (0 starts with the line).
    Cue { block: String, word: usize },
    /// Run the block after its line.
    Hold { block: String },
    /// Put the block after line `after`, holding or cued at `word`.
    Move {
        block: String,
        after: String,
        word: Option<usize>,
    },
    /// Multiply the block's stretch by `by`; a stretch that comes to 1
    /// is removed.
    Stretch { block: String, by: f64 },
    /// Say line `line` as `text`, keeping its attributes.
    Reword { line: String, text: String },
}

/// Applies `edit` to the script `src`.
pub fn apply(src: &str, edit: &Edit) -> Result<String, String> {
    let map = Map::of(src)?;
    match edit {
        Edit::Cue { block, word } => {
            let b = map.block(block)?;
            let line = map.paired(b)?;
            let info = cued(&b.info, &line.text, *word)?;
            Ok(replace(src, b.info_range.clone(), &info))
        }
        Edit::Hold { block } => {
            let b = map.block(block)?;
            let info = with(&b.info, &[("policy", None), ("cue", None), ("align", None)]);
            Ok(replace(src, b.info_range.clone(), &info))
        }
        Edit::Stretch { block, by } => {
            let b = map.block(block)?;
            let now: f64 = attr(&b.info, "stretch")
                .and_then(|s| s.parse().ok())
                .unwrap_or(1.0);
            let next = (now * by * 100.0).round() / 100.0;
            if next <= 0.0 {
                return Err(format!("a stretch of {next} is no length at all"));
            }
            let value = (next != 1.0).then(|| format!("{next}"));
            let info = with(&b.info, &[("stretch", value.as_deref())]);
            Ok(replace(src, b.info_range.clone(), &info))
        }
        Edit::Reword { line, text } => {
            let range = map.line(line)?.range.clone();
            let paragraph = &src[range.clone()];
            // Its `{#id …}`, after the words.
            let attrs = paragraph
                .ends_with('}')
                .then(|| paragraph.rfind('{'))
                .flatten()
                .map_or("", |at| &paragraph[at..]);
            let reworded = if attrs.is_empty() {
                text.trim().to_string()
            } else {
                format!("{} {attrs}", text.trim())
            };
            Ok(replace(src, range, &reworded))
        }
        Edit::Move { block, after, word } => {
            let b = map.block(block)?;
            let line = map.line(after)?;
            let info = match word {
                Some(w) => cued(&b.info, &line.text, *w)?,
                None => with(&b.info, &[("policy", None), ("cue", None), ("align", None)]),
            };
            let fence = format!(
                "{}{info}{}",
                &src[b.range.start..b.info_range.start],
                &src[b.info_range.end..b.range.end]
            );
            // Cut the block with the blank lines before it, then insert it
            // after the line's paragraph, a blank line between.
            let cut = src[..b.range.start].trim_end_matches('\n').len() + 1..b.range.end;
            let insert_at = line.range.end;
            let piece = format!("\n\n{}", fence.trim_end_matches('\n'));
            let mut out = String::with_capacity(src.len());
            if insert_at < cut.start {
                out.push_str(&src[..insert_at]);
                out.push_str(&piece);
                out.push_str(&src[insert_at..cut.start]);
                out.push_str(&src[cut.end..]);
            } else {
                out.push_str(&src[..cut.start]);
                out.push_str(&src[cut.end..insert_at]);
                out.push_str(&piece);
                out.push_str(&src[insert_at..]);
            }
            Ok(out)
        }
    }
}

/// Where the script's lines and blocks are, by id.
struct Map {
    lines: Vec<LineAt>,
    blocks: Vec<BlockAt>,
}

struct LineAt {
    id: String,
    text: String,
    /// The paragraph, up to its last character.
    range: Range<usize>,
}

struct BlockAt {
    id: String,
    info: String,
    /// The attributes after ```` ```teleprompt ````, in the fence's line.
    info_range: Range<usize>,
    /// The fence, opening line to closing line, with its newline.
    range: Range<usize>,
    /// The line right before it, which it pairs with.
    paired: Option<String>,
}

impl Map {
    fn of(src: &str) -> Result<Map, String> {
        let script = parse_script(src).map_err(|d| {
            let first = d.0.first().map_or(String::new(), |d| d.message.clone());
            format!("the script does not parse: {first}")
        })?;
        let starts: Vec<usize> = std::iter::once(0)
            .chain(src.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        let at = |line: usize| starts.get(line - 1).copied().unwrap_or(src.len());
        let mut map = Map {
            lines: Vec::new(),
            blocks: Vec::new(),
        };
        for chapter in &script.chapters {
            let mut previous: Option<String> = None;
            for node in &chapter.nodes {
                match node {
                    Node::Line(l) => {
                        let start = at(l.span.line);
                        let end = (start + l.span.len).min(src.len());
                        let id = l.id.to_string();
                        map.lines.push(LineAt {
                            id: id.clone(),
                            text: l.text.clone(),
                            range: start..src[..end].trim_end().len().max(start),
                        });
                        previous = Some(id);
                    }
                    Node::ActionBlock(b) => {
                        let start = at(b.span.line);
                        let end = (start + b.span.len).min(src.len());
                        let end = src[end..].find('\n').map_or(src.len(), |i| end + i + 1);
                        let fence_line =
                            &src[start..src[start..].find('\n').map_or(src.len(), |i| start + i)];
                        let info_start = start
                            + fence_line
                                .find("teleprompt")
                                .map_or(0, |i| i + "teleprompt".len());
                        let info_end = start + fence_line.len();
                        map.blocks.push(BlockAt {
                            id: b.id.to_string(),
                            info: src[info_start..info_end].trim().to_string(),
                            info_range: info_start..info_end,
                            range: start..end,
                            paired: previous.take(),
                        });
                    }
                    Node::Directive(_) => previous = None,
                }
            }
        }
        Ok(map)
    }

    fn block(&self, id: &str) -> Result<&BlockAt, String> {
        self.blocks
            .iter()
            .find(|b| b.id == id)
            .ok_or_else(|| format!("no block `{id}` in the script"))
    }

    fn line(&self, id: &str) -> Result<&LineAt, String> {
        self.lines
            .iter()
            .find(|l| l.id == id)
            .ok_or_else(|| format!("no line `{id}` in the script"))
    }

    fn paired(&self, b: &BlockAt) -> Result<&LineAt, String> {
        let id = b.paired.as_deref().ok_or_else(|| {
            format!(
                "block `{}` follows no line, so it has nothing to start with",
                b.id
            )
        })?;
        self.line(id)
    }
}

/// `info` running with a line whose text is `text`, from its `word`th
/// word: named by the shortest phrase from there, of two words or more,
/// that the line says once.
fn cued(info: &str, text: &str, word: usize) -> Result<String, String> {
    if word == 0 {
        return Ok(with(
            info,
            &[
                ("policy", Some("concurrent")),
                ("cue", None),
                ("align", None),
            ],
        ));
    }
    let words: Vec<(usize, &str)> = text
        .split_whitespace()
        .map(|w| (w.as_ptr() as usize - text.as_ptr() as usize, w))
        .collect();
    let Some(&(from, _)) = words.get(word) else {
        return Err(format!(
            "the line has {} words, not {}",
            words.len(),
            word + 1
        ));
    };
    // Two words at least, where there are two, since one short word is
    // said inside others; but not across the end of a sentence.
    let ends_sentence = words[word].1.ends_with(['.', '!', '?']);
    let shortest = if ends_sentence { word } else { word + 1 };
    let phrase = (shortest.min(words.len() - 1)..words.len())
        .map(|last| {
            let (at, w) = words[last];
            text[from..at + w.len()].trim_end_matches([',', '.', ';', ':', '!', '?'])
        })
        .find(|p| !p.is_empty() && text.matches(*p).count() == 1 && !p.contains('"'))
        .ok_or("no phrase from there is said only once in the line")?;
    Ok(with(
        info,
        &[
            ("policy", Some("concurrent")),
            ("cue", Some(phrase)),
            ("align", None),
        ],
    ))
}

/// A fence's attributes, each a `key=value` or `key="quoted value"`.
fn attrs(info: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = info.trim();
    while !rest.is_empty() {
        let (key, after) = rest.split_once('=').unwrap_or((rest, ""));
        let (value, after) = match after.strip_prefix('"') {
            Some(quoted) => match quoted.find('"') {
                Some(end) => (&quoted[..end], &quoted[end + 1..]),
                None => (quoted, ""),
            },
            None => after.split_once(char::is_whitespace).unwrap_or((after, "")),
        };
        out.push((key.trim().to_string(), value.to_string()));
        rest = after.trim_start();
    }
    out
}

fn attr(info: &str, key: &str) -> Option<String> {
    attrs(info)
        .into_iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
}

/// `info` with each key set to its value, or removed for `None`; kept in
/// its place when it was there, added at the end when it was not.
fn with(info: &str, changes: &[(&str, Option<&str>)]) -> String {
    let mut list = attrs(info);
    for (key, value) in changes {
        match (list.iter().position(|(k, _)| k == key), value) {
            (Some(i), Some(v)) => list[i].1 = (*v).to_string(),
            (Some(i), None) => {
                list.remove(i);
            }
            (None, Some(v)) => list.push(((*key).to_string(), (*v).to_string())),
            (None, None) => {}
        }
    }
    let written: Vec<String> = list
        .iter()
        .map(|(k, v)| {
            if v.contains(char::is_whitespace) || v.is_empty() {
                format!("{k}=\"{v}\"")
            } else {
                format!("{k}={v}")
            }
        })
        .collect();
    format!(" {}", written.join(" "))
}

fn replace(src: &str, range: Range<usize>, with: &str) -> String {
    format!("{}{with}{}", &src[..range.start], &src[range.end..])
}
