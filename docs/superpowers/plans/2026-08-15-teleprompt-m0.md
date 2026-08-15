# teleprompt M0 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the teleprompt feedback loop — parse a Markdown script, compute a timeline from narration durations, and diff it against the committed one — with no video rendering and no external runtime.

**Architecture:** A Rust workspace of five crates. `teleprompt-core` parses Markdown into an AST and resolves configuration; `teleprompt-scene` defines the compile-time half of the scene contract plus a deterministic `mock` adapter; `teleprompt-voice` defines the backend trait plus a `null` backend that estimates duration from word count; `teleprompt-schedule` is a pure function from (program, audio durations, span durations) to a `Timeline`, plus timeline diffing; `teleprompt-cli` is a thin clap shell. Everything is pure Rust with no network, no browser, no ffmpeg, and no audio device.

**Tech Stack:** Rust 2021, `pulldown-cmark` (CommonMark), `serde` + `serde_json` + `serde_yaml` + `toml`, `blake3`, `thiserror`, `clap` (derive), `insta` (dev-only, golden snapshots).

**Spec:** `docs/superpowers/specs/2026-08-15-teleprompt-design.md`

## Global Constraints

- Rust edition 2021, MSRV 1.75.
- `teleprompt-core` and `teleprompt-schedule` MUST have no async runtime, no network, and no filesystem access beyond reading the script. Adding `tokio`, `reqwest`, or similar to either crate is a plan violation.
- M0 ships no rendering. Do not add `ffmpeg` invocation, capture, or compositing.
- M0 ships no external runtime. Do not add Node, Playwright, or a browser dependency.
- Every command supports `--format json` with a stable schema.
- Exit codes: `0` success · `1` runtime failure · `2` validation error · `3` timeline drift under `--exit-code` · `4` voice downgrade under `--strict-voice`.
- Hashes are BLAKE3, rendered as the first 6 hex characters in human output and full hex in JSON.
- Default padding: `lead_in` 150 ms, `tail` 150 ms.
- Default policy bounds: `max_stretch` 3.0, `min_stretch` 0.33, `max_speedup` 2.0.
- Default transition: `crossfade`, `duration: auto`, `min_ms` 0, `max_ms` 600.
- Unknown attribute keys are an error, never a silent ignore.
- Timelines are committed to `timelines/<script>.<locale>.json`.

**Known dependency caveat:** `serde_yaml` 0.9 is archived upstream but stable and widely used; it is the M0 choice for front matter. Migrating to a maintained fork is a follow-up, not part of this plan.

### Two refinements this plan makes to the spec

**The `Scene` trait splits in half.** §7.2 defines one trait covering both compile-time (`validate`, `spans`, `estimate`) and execution (`prepare`, `execute`, `teardown`). M0 needs only the compile-time half and cannot meaningfully implement or test the other. This plan builds `SceneCompiler` now and leaves `SceneExecutor` for M1, with `Scene: SceneCompiler + SceneExecutor` assembled there. No spec requirement is dropped.

**A `teleprompt-compile` crate joins the workspace.** §12's crate table has no home for the seam where parsed programs meet scene adapters and voice backends. Putting that glue in any of the four would force a dependency between crates the spec keeps independent — `schedule` would have to know about `scene`, or `core` about both. A small crate that depends on all four and is depended on only by `cli` keeps every arrow pointing one way. Fold this into the spec's §12 table when M0 lands.

## File Structure

```
Cargo.toml                                  workspace manifest
crates/
  teleprompt-core/
    src/lib.rs                              re-exports
    src/error.rs                            SourceSpan, Diagnostic, ParseError
    src/hash.rs                             Hash newtype over blake3
    src/ast.rs                              Script, Chapter, Node, Segment, ActionBlock, Directive
    src/attrs.rs                            Attributes map, key whitelists, typed getters
    src/ident.rs                            SegmentId, BlockId, slugging, derived IDs
    src/parse.rs                            CommonMark -> Script
    src/config.rs                           Config types, six-level merge
    src/program.rs                          Program, resolve()
  teleprompt-scene/
    src/lib.rs                              re-exports
    src/contract.rs                         SceneCompiler, BlockSource, Validated, Span, Measured
    src/registry.rs                         SceneRegistry, scene->adapter binding
    src/mock.rs                             deterministic mock adapter
  teleprompt-voice/
    src/lib.rs                              re-exports
    src/contract.rs                         VoiceBackend, VoiceCapabilities, SynthRequest/Result
    src/source.rs                           VoiceSource, fallback ladder
    src/null.rs                             null backend, duration estimation
  teleprompt-schedule/
    src/lib.rs                              re-exports
    src/beat.rs                             Beat, NarrationInput, ActionInput
    src/policy.rs                           Policy, Align, per-policy layout
    src/timeline.rs                         Timeline, Entry, JSON schema
    src/schedule.rs                         schedule() — the pure function
    src/diff.rs                             TimelineDiff, compare(), prose rendering
  teleprompt-compile/
    src/lib.rs                              program + adapters + voice -> Timeline
  teleprompt-cli/
    src/main.rs                             clap entry, exit codes
    src/cmd/new.rs                          scaffold
    src/cmd/check.rs                        parse + validate
    src/cmd/plan.rs                         compile timeline
    src/cmd/diff.rs                         compare against committed
    src/cmd/doctor.rs                       environment report
    src/output.rs                           human vs json formatting
tests/fixtures/                             golden scripts + expected output
```

Why this split: `core` holds everything that turns text into typed data and nothing else. `schedule` holds the most intricate logic in the project and is a pure function, so it is table-testable with no fixtures. `scene` and `voice` are contracts plus one trivial implementation each, which is what proves the contracts are usable without proving anything about browsers or TTS. `cli` holds no logic — every subcommand is a call into a library function that returns data, and formatting is separate from computation so `--format json` is not an afterthought.

---

### Task 1: Workspace and error types

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`, `.gitignore`
- Create: `crates/teleprompt-core/Cargo.toml`, `crates/teleprompt-core/src/lib.rs`
- Create: `crates/teleprompt-core/src/error.rs`, `crates/teleprompt-core/src/hash.rs`
- Test: inline `#[cfg(test)]` in `error.rs` and `hash.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `SourceSpan { line: usize, column: usize, len: usize }`; `Diagnostic { severity: Severity, message: String, span: Option<SourceSpan>, help: Option<String> }`; `Severity::{Error, Warning}`; `Hash([u8; 32])` with `Hash::of(&[u8]) -> Hash`, `Hash::short(&self) -> String` (6 hex chars), `Display` as full hex.

- [ ] **Step 1: Create the workspace manifest**

```toml
# Cargo.toml
[workspace]
resolver = "2"
members = ["crates/*"]

[workspace.package]
edition = "2021"
rust-version = "1.75"
license = "MIT"

[workspace.dependencies]
blake3 = "1"
clap = { version = "4", features = ["derive"] }
insta = { version = "1", features = ["json"] }
pulldown-cmark = { version = "0.11", default-features = false }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
thiserror = "1"
toml = "0.8"
```

```toml
# rust-toolchain.toml
[toolchain]
channel = "1.75"
components = ["rustfmt", "clippy"]
```

```
# .gitignore
/target
.teleprompt/
build/
```

- [ ] **Step 2: Write the failing tests**

```rust
// crates/teleprompt-core/src/hash.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_deterministic() {
        assert_eq!(Hash::of(b"welcome"), Hash::of(b"welcome"));
    }

    #[test]
    fn hash_distinguishes_content() {
        assert_ne!(Hash::of(b"welcome"), Hash::of(b"welcome "));
    }

    #[test]
    fn short_form_is_six_hex_chars() {
        let s = Hash::of(b"welcome").short();
        assert_eq!(s.len(), 6);
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
```

```rust
// crates/teleprompt-core/src/error.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_renders_file_line_and_column() {
        let d = Diagnostic::error("unknown attribute key `polcy`")
            .at(SourceSpan { line: 12, column: 5, len: 5 })
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
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p teleprompt-core`
Expected: FAIL — `cannot find type Hash`, `cannot find type Diagnostic`.

- [ ] **Step 4: Implement hash and error types**

```rust
// crates/teleprompt-core/src/hash.rs
use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hash([u8; 32]);

impl Hash {
    pub fn of(bytes: &[u8]) -> Self {
        Hash(*blake3::hash(bytes).as_bytes())
    }

    pub fn short(&self) -> String {
        self.to_string()[..6].to_string()
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in &self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash({})", self.short())
    }
}

impl serde::Serialize for Hash {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}
```

```rust
// crates/teleprompt-core/src/error.rs
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
        Self { severity: Severity::Error, message: message.into(), span: None, help: None }
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Self { severity: Severity::Warning, message: message.into(), span: None, help: None }
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
```

```rust
// crates/teleprompt-core/src/lib.rs
pub mod error;
pub mod hash;

pub use error::{Diagnostic, Diagnostics, Severity, SourceSpan};
pub use hash::Hash;
```

```toml
# crates/teleprompt-core/Cargo.toml
[package]
name = "teleprompt-core"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
blake3.workspace = true
pulldown-cmark.workspace = true
serde.workspace = true
serde_yaml.workspace = true
thiserror.workspace = true
toml.workspace = true
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p teleprompt-core`
Expected: PASS, 5 tests.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml rust-toolchain.toml .gitignore crates/teleprompt-core
git commit -m "feat(core): workspace, diagnostics, and content hashing"
```

---

### Task 2: Markdown parsing into an AST

**Files:**
- Create: `crates/teleprompt-core/src/ast.rs`, `crates/teleprompt-core/src/parse.rs`
- Modify: `crates/teleprompt-core/src/lib.rs`
- Test: `crates/teleprompt-core/tests/parse.rs`

**Interfaces:**
- Consumes: `Diagnostic`, `Diagnostics`, `SourceSpan` from Task 1.
- Produces: `parse_script(src: &str) -> Result<Script, Diagnostics>`; `Script { front_matter: String, chapters: Vec<Chapter> }`; `Chapter { title: String, slug: String, nodes: Vec<Node> }`; `Node::{Segment, ActionBlock, Directive}`; `Segment { id: Option<String>, text: String, raw_attrs: String, span: SourceSpan }`; `ActionBlock { info: String, body: String, span: SourceSpan }`; `Directive::Pause(u64)`.

Note: `Segment.id` is `Option` here — Task 3 assigns derived IDs. `raw_attrs` and `info` are unparsed strings — Task 4 parses them. Keeping parsing layered means each layer is testable alone.

- [ ] **Step 1: Write the failing tests**

```rust
// crates/teleprompt-core/tests/parse.rs
use teleprompt_core::ast::{Directive, Node};
use teleprompt_core::parse::parse_script;

const BASIC: &str = r#"---
teleprompt: 1
---

# Quickstart

Welcome to Acme. {#welcome}

```teleprompt scene=browser
await page.goto('/');
```

Second paragraph here.
"#;

#[test]
fn extracts_front_matter() {
    let s = parse_script(BASIC).unwrap();
    assert!(s.front_matter.contains("teleprompt: 1"));
}

#[test]
fn heading_becomes_chapter_with_slug() {
    let s = parse_script(BASIC).unwrap();
    assert_eq!(s.chapters.len(), 1);
    assert_eq!(s.chapters[0].title, "Quickstart");
    assert_eq!(s.chapters[0].slug, "quickstart");
}

#[test]
fn paragraphs_become_segments_and_fences_become_action_blocks() {
    let s = parse_script(BASIC).unwrap();
    let kinds: Vec<&str> = s.chapters[0]
        .nodes
        .iter()
        .map(|n| match n {
            Node::Segment(_) => "segment",
            Node::ActionBlock(_) => "action",
            Node::Directive(_) => "directive",
        })
        .collect();
    assert_eq!(kinds, ["segment", "action", "segment"]);
}

#[test]
fn action_block_body_is_verbatim() {
    let s = parse_script(BASIC).unwrap();
    let Node::ActionBlock(b) = &s.chapters[0].nodes[1] else { panic!("expected action block") };
    assert_eq!(b.body.trim(), "await page.goto('/');");
    assert_eq!(b.info, "scene=browser");
}

#[test]
fn segment_text_excludes_the_attribute_suffix() {
    let s = parse_script(BASIC).unwrap();
    let Node::Segment(seg) = &s.chapters[0].nodes[0] else { panic!("expected segment") };
    assert_eq!(seg.text, "Welcome to Acme.");
    assert_eq!(seg.raw_attrs, "#welcome");
}

#[test]
fn non_teleprompt_fences_are_ignored() {
    let src = "# C\n\nText.\n\n```rust\nfn main() {}\n```\n";
    let s = parse_script(src).unwrap();
    assert_eq!(s.chapters[0].nodes.len(), 1);
}

#[test]
fn lists_and_tables_are_ignored() {
    let src = "# C\n\nText.\n\n- a\n- b\n\n| x | y |\n|---|---|\n| 1 | 2 |\n";
    let s = parse_script(src).unwrap();
    assert_eq!(s.chapters[0].nodes.len(), 1);
}

#[test]
fn html_comment_paragraph_is_a_directive() {
    let src = "# C\n\n<!-- teleprompt: pause 800ms -->\n";
    let s = parse_script(src).unwrap();
    assert!(matches!(s.chapters[0].nodes[0], Node::Directive(Directive::Pause(800))));
}

#[test]
fn inline_markup_is_normalised_for_speech() {
    let src = "# C\n\nUse *the* `--watch` [flag](https://x.dev).\n";
    let s = parse_script(src).unwrap();
    let Node::Segment(seg) = &s.chapters[0].nodes[0] else { panic!("expected segment") };
    assert_eq!(seg.text, "Use the --watch flag.");
}

#[test]
fn content_before_any_heading_is_an_error() {
    let src = "Orphan paragraph.\n\n# C\n\nText.\n";
    let err = parse_script(src).unwrap_err();
    assert!(err.0[0].message.contains("before the first heading"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p teleprompt-core --test parse`
Expected: FAIL — `unresolved import teleprompt_core::parse`.

- [ ] **Step 3: Implement the AST**

```rust
// crates/teleprompt-core/src/ast.rs
use crate::SourceSpan;

#[derive(Debug, Clone)]
pub struct Script {
    pub front_matter: String,
    pub chapters: Vec<Chapter>,
}

#[derive(Debug, Clone)]
pub struct Chapter {
    pub title: String,
    pub slug: String,
    pub nodes: Vec<Node>,
}

#[derive(Debug, Clone)]
pub enum Node {
    Segment(Segment),
    ActionBlock(ActionBlock),
    Directive(Directive),
}

#[derive(Debug, Clone)]
pub struct Segment {
    pub id: Option<String>,
    pub text: String,
    pub raw_attrs: String,
    pub span: SourceSpan,
}

#[derive(Debug, Clone)]
pub struct ActionBlock {
    pub id: Option<String>,
    pub info: String,
    pub body: String,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Directive {
    Pause(u64),
}

pub fn slugify(title: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in title.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_end_matches('-').to_string()
}
```

- [ ] **Step 4: Implement the parser**

```rust
// crates/teleprompt-core/src/parse.rs
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use crate::ast::{slugify, ActionBlock, Chapter, Directive, Node, Script, Segment};
use crate::{Diagnostic, Diagnostics, SourceSpan};

const FENCE_TAG: &str = "teleprompt";

pub fn parse_script(src: &str) -> Result<Script, Diagnostics> {
    let (front_matter, body, body_offset) = split_front_matter(src);
    let mut diags = Vec::new();
    let chapters = parse_body(body, body_offset, &mut diags);

    let d = Diagnostics(diags);
    if d.has_errors() {
        return Err(d);
    }
    Ok(Script { front_matter, chapters })
}

fn split_front_matter(src: &str) -> (String, &str, usize) {
    let Some(rest) = src.strip_prefix("---\n") else {
        return (String::new(), src, 0);
    };
    match rest.find("\n---\n") {
        Some(end) => {
            let fm = rest[..end].to_string();
            let after = &rest[end + 5..];
            let lines = 1 + rest[..end + 5].lines().count();
            (fm, after, lines)
        }
        None => (String::new(), src, 0),
    }
}

fn parse_body(body: &str, line_offset: usize, diags: &mut Vec<Diagnostic>) -> Vec<Chapter> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    let parser = Parser::new_ext(body, opts).into_offset_iter();

    let mut chapters: Vec<Chapter> = Vec::new();
    let mut state = State::Idle;
    let mut text = String::new();
    let mut fence_info = String::new();

    for (event, range) in parser {
        let line = line_offset + body[..range.start].lines().count();
        let span = SourceSpan { line, column: 1, len: range.len() };

        match event {
            Event::Start(Tag::Heading { .. }) => {
                state = State::Heading;
                text.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                let title = text.trim().to_string();
                chapters.push(Chapter { slug: slugify(&title), title, nodes: Vec::new() });
                state = State::Idle;
                text.clear();
            }
            Event::Start(Tag::Paragraph) => {
                state = State::Paragraph;
                text.clear();
            }
            Event::End(TagEnd::Paragraph) => {
                let raw = text.trim().to_string();
                if !raw.is_empty() {
                    push_node(&mut chapters, paragraph_node(&raw, span), span, diags);
                }
                state = State::Idle;
                text.clear();
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                fence_info = info.to_string();
                state = if fence_info.split_whitespace().next() == Some(FENCE_TAG) {
                    State::ActionBlock
                } else {
                    State::Idle
                };
                text.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                if matches!(state, State::ActionBlock) {
                    let info = fence_info
                        .strip_prefix(FENCE_TAG)
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    let node = Node::ActionBlock(ActionBlock {
                        id: None,
                        info,
                        body: text.clone(),
                        span,
                    });
                    push_node(&mut chapters, Some(node), span, diags);
                }
                state = State::Idle;
                text.clear();
            }
            Event::Text(t) | Event::Code(t) => {
                if !matches!(state, State::Idle) {
                    text.push_str(&t);
                }
            }
            Event::Html(h) | Event::InlineHtml(h) => {
                if let Some(node) = directive_from_html(&h) {
                    push_node(&mut chapters, Some(node), span, diags);
                }
            }
            _ => {}
        }
    }

    chapters
}

enum State {
    Idle,
    Heading,
    Paragraph,
    ActionBlock,
}

fn paragraph_node(raw: &str, span: SourceSpan) -> Option<Node> {
    if let Some(node) = directive_from_html(raw) {
        return Some(node);
    }
    let (text, raw_attrs) = split_attr_suffix(raw);
    Some(Node::Segment(Segment {
        id: None,
        text: text.trim().to_string(),
        raw_attrs,
        span,
    }))
}

/// Splits a trailing `{...}` attribute suffix off a paragraph.
fn split_attr_suffix(raw: &str) -> (&str, String) {
    let trimmed = raw.trim_end();
    if !trimmed.ends_with('}') {
        return (raw, String::new());
    }
    match trimmed.rfind('{') {
        Some(open) => (&trimmed[..open], trimmed[open + 1..trimmed.len() - 1].to_string()),
        None => (raw, String::new()),
    }
}

fn directive_from_html(html: &str) -> Option<Node> {
    let inner = html.trim().strip_prefix("<!--")?.strip_suffix("-->")?.trim();
    let rest = inner.strip_prefix("teleprompt:")?.trim();
    let ms = rest.strip_prefix("pause")?.trim().strip_suffix("ms")?.trim();
    ms.parse().ok().map(|n| Node::Directive(Directive::Pause(n)))
}

fn push_node(
    chapters: &mut Vec<Chapter>,
    node: Option<Node>,
    span: SourceSpan,
    diags: &mut Vec<Diagnostic>,
) {
    let Some(node) = node else { return };
    match chapters.last_mut() {
        Some(ch) => ch.nodes.push(node),
        None => diags.push(
            Diagnostic::error("content appears before the first heading")
                .at(span)
                .with_help("every segment and action block must belong to a chapter"),
        ),
    }
}
```

```rust
// crates/teleprompt-core/src/lib.rs — add
pub mod ast;
pub mod parse;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p teleprompt-core`
Expected: PASS, 15 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/teleprompt-core
git commit -m "feat(core): parse Markdown scripts into chapters, segments, and action blocks"
```

---

### Task 3: Segment identity

**Files:**
- Create: `crates/teleprompt-core/src/ident.rs`
- Modify: `crates/teleprompt-core/src/lib.rs`, `crates/teleprompt-core/src/parse.rs`
- Test: `crates/teleprompt-core/tests/ident.rs`

**Interfaces:**
- Consumes: `Script`, `Chapter`, `Node`, `Segment`, `ActionBlock` from Task 2.
- Produces: `assign_ids(script: &mut Script) -> Vec<Diagnostic>`; `SegmentId(String)` and `BlockId(String)` newtypes with `as_str()`; `IdOrigin::{Explicit, Derived}` recorded on each segment as `Segment.id_origin`.

Derived IDs are `<chapter-slug>-<n>`, `n` counting segments within the chapter from 1. Action blocks derive `<segment-id>-a`, or `<chapter-slug>-b<n>` when no segment precedes them. Explicit `{#id}` wins. Duplicate IDs are an error.

- [ ] **Step 1: Write the failing tests**

```rust
// crates/teleprompt-core/tests/ident.rs
use teleprompt_core::ast::Node;
use teleprompt_core::ident::{assign_ids, IdOrigin};
use teleprompt_core::parse::parse_script;

fn ids(src: &str) -> Vec<String> {
    let mut s = parse_script(src).unwrap();
    let diags = assign_ids(&mut s);
    assert!(!diags.iter().any(|d| d.is_error()), "unexpected errors: {diags:?}");
    s.chapters
        .iter()
        .flat_map(|c| c.nodes.iter())
        .filter_map(|n| match n {
            Node::Segment(seg) => seg.id.clone(),
            Node::ActionBlock(b) => b.id.clone(),
            Node::Directive(_) => None,
        })
        .collect()
}

#[test]
fn derived_ids_number_segments_within_a_chapter() {
    let src = "# Quick Start\n\nOne.\n\nTwo.\n";
    assert_eq!(ids(src), ["quick-start-1", "quick-start-2"]);
}

#[test]
fn numbering_restarts_per_chapter() {
    let src = "# A\n\nOne.\n\n# B\n\nTwo.\n";
    assert_eq!(ids(src), ["a-1", "b-1"]);
}

#[test]
fn explicit_id_wins_and_does_not_consume_an_ordinal() {
    let src = "# A\n\nOne. {#welcome}\n\nTwo.\n";
    assert_eq!(ids(src), ["welcome", "a-2"]);
}

#[test]
fn action_block_derives_from_the_preceding_segment() {
    let src = "# A\n\nOne.\n\n```teleprompt scene=mock\nwait 100ms\n```\n";
    assert_eq!(ids(src), ["a-1", "a-1-a"]);
}

#[test]
fn leading_action_block_derives_from_the_chapter() {
    let src = "# A\n\n```teleprompt scene=mock\nwait 100ms\n```\n\nOne.\n";
    assert_eq!(ids(src), ["a-b1", "a-1"]);
}

#[test]
fn id_origin_is_recorded() {
    let src = "# A\n\nOne. {#welcome}\n\nTwo.\n";
    let mut s = parse_script(src).unwrap();
    assign_ids(&mut s);
    let origins: Vec<IdOrigin> = s.chapters[0]
        .nodes
        .iter()
        .filter_map(|n| match n {
            Node::Segment(seg) => Some(seg.id_origin),
            _ => None,
        })
        .collect();
    assert_eq!(origins, [IdOrigin::Explicit, IdOrigin::Derived]);
}

#[test]
fn duplicate_explicit_ids_are_an_error() {
    let src = "# A\n\nOne. {#dup}\n\nTwo. {#dup}\n";
    let mut s = parse_script(src).unwrap();
    let diags = assign_ids(&mut s);
    assert!(diags.iter().any(|d| d.is_error() && d.message.contains("duplicate segment id `dup`")));
}

#[test]
fn explicit_id_colliding_with_a_derived_id_is_an_error() {
    let src = "# A\n\nOne.\n\nTwo. {#a-1}\n";
    let mut s = parse_script(src).unwrap();
    let diags = assign_ids(&mut s);
    assert!(diags.iter().any(|d| d.is_error() && d.message.contains("duplicate segment id `a-1`")));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p teleprompt-core --test ident`
Expected: FAIL — `unresolved import teleprompt_core::ident`.

- [ ] **Step 3: Add `id_origin` to the AST**

```rust
// crates/teleprompt-core/src/ast.rs — add to Segment
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdOrigin {
    Explicit,
    Derived,
}

// Segment gains one field:
//   pub id_origin: IdOrigin,
// Initialise it to IdOrigin::Derived in parse.rs where Segment is constructed.
```

- [ ] **Step 4: Implement identity assignment**

```rust
// crates/teleprompt-core/src/ident.rs
use std::collections::HashSet;

use crate::ast::{IdOrigin, Node, Script};
use crate::Diagnostic;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SegmentId(pub String);

impl SegmentId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(pub String);

impl BlockId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub use crate::ast::IdOrigin as _IdOriginReexport;

pub fn assign_ids(script: &mut Script) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for chapter in &mut script.chapters {
        let slug = chapter.slug.clone();
        let mut seg_n = 0usize;
        let mut block_n = 0usize;
        let mut last_segment: Option<String> = None;

        for node in &mut chapter.nodes {
            match node {
                Node::Segment(seg) => {
                    seg_n += 1;
                    let (id, origin) = match &seg.id {
                        Some(explicit) => (explicit.clone(), IdOrigin::Explicit),
                        None => (format!("{slug}-{seg_n}"), IdOrigin::Derived),
                    };
                    if !seen.insert(id.clone()) {
                        diags.push(
                            Diagnostic::error(format!("duplicate segment id `{id}`"))
                                .at(seg.span)
                                .with_help("give one of them an explicit unique `{#id}`"),
                        );
                    }
                    last_segment = Some(id.clone());
                    seg.id = Some(id);
                    seg.id_origin = origin;
                }
                Node::ActionBlock(block) => {
                    if block.id.is_none() {
                        block.id = Some(match &last_segment {
                            Some(seg) => format!("{seg}-a"),
                            None => {
                                block_n += 1;
                                format!("{slug}-b{block_n}")
                            }
                        });
                    }
                }
                Node::Directive(_) => {}
            }
        }
    }

    diags
}
```

Note on `seen`: explicit IDs are inserted as encountered, so an explicit `{#a-1}` appearing after the derived `a-1` collides and is reported. Both orderings are covered by the tests.

```rust
// crates/teleprompt-core/src/lib.rs — add
pub mod ident;
```

- [ ] **Step 5: Populate explicit IDs during parsing**

In `parse.rs`, `split_attr_suffix` already yields `raw_attrs`. Set `Segment.id` when the suffix starts with `#`:

```rust
// in paragraph_node(), replacing the Segment construction
let (text, raw_attrs) = split_attr_suffix(raw);
let id = raw_attrs
    .split_whitespace()
    .next()
    .and_then(|t| t.strip_prefix('#'))
    .map(str::to_string);
Some(Node::Segment(Segment {
    id,
    id_origin: IdOrigin::Derived,
    text: text.trim().to_string(),
    raw_attrs,
    span,
}))
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p teleprompt-core`
Expected: PASS, 23 tests.

- [ ] **Step 7: Commit**

```bash
git add crates/teleprompt-core
git commit -m "feat(core): assign stable segment and block identifiers"
```

---

### Task 4: Attribute grammar and validation

**Files:**
- Create: `crates/teleprompt-core/src/attrs.rs`
- Modify: `crates/teleprompt-core/src/lib.rs`
- Test: `crates/teleprompt-core/tests/attrs.rs`

**Interfaces:**
- Consumes: `Diagnostic`, `SourceSpan` from Task 1.
- Produces: `Attributes` (ordered map) with `parse_attrs(raw: &str, allowed: &[&str], span: SourceSpan) -> (Attributes, Vec<Diagnostic>)`, `Attributes::get(&self, key: &str) -> Option<&str>`, `Attributes::get_f64`, `Attributes::get_ms`; constants `SEGMENT_KEYS` and `BLOCK_KEYS`.

`SEGMENT_KEYS = ["voice.source", "voice.backend", "voice.voice", "voice.speed", "lead_in", "tail", "lang"]`
`BLOCK_KEYS = ["scene", "include", "policy", "align", "id", "max_speedup", "max_stretch", "min_stretch"]`

- [ ] **Step 1: Write the failing tests**

```rust
// crates/teleprompt-core/tests/attrs.rs
use teleprompt_core::attrs::{parse_attrs, BLOCK_KEYS, SEGMENT_KEYS};
use teleprompt_core::SourceSpan;

const SPAN: SourceSpan = SourceSpan { line: 1, column: 1, len: 0 };

#[test]
fn parses_key_value_pairs() {
    let (a, d) = parse_attrs("policy=concurrent align=start", BLOCK_KEYS, SPAN);
    assert!(d.is_empty());
    assert_eq!(a.get("policy"), Some("concurrent"));
    assert_eq!(a.get("align"), Some("start"));
}

#[test]
fn ignores_the_leading_id_token() {
    let (a, d) = parse_attrs("#welcome voice.source=recorded", SEGMENT_KEYS, SPAN);
    assert!(d.is_empty());
    assert_eq!(a.get("voice.source"), Some("recorded"));
}

#[test]
fn accepts_quoted_values_with_spaces() {
    let (a, d) = parse_attrs(r#"include="scripts/my demo.spec.ts""#, BLOCK_KEYS, SPAN);
    assert!(d.is_empty());
    assert_eq!(a.get("include"), Some("scripts/my demo.spec.ts"));
}

#[test]
fn unknown_key_is_an_error_with_a_suggestion() {
    let (_, d) = parse_attrs("polcy=hold", BLOCK_KEYS, SPAN);
    assert_eq!(d.len(), 1);
    assert!(d[0].is_error());
    assert!(d[0].message.contains("unknown attribute key `polcy`"));
    assert_eq!(d[0].help.as_deref(), Some("did you mean `policy`?"));
}

#[test]
fn unknown_key_with_no_near_match_has_no_suggestion() {
    let (_, d) = parse_attrs("zzzzzz=1", BLOCK_KEYS, SPAN);
    assert_eq!(d.len(), 1);
    assert!(d[0].help.is_none());
}

#[test]
fn a_key_valid_elsewhere_is_still_rejected_here() {
    let (_, d) = parse_attrs("policy=hold", SEGMENT_KEYS, SPAN);
    assert!(d[0].message.contains("unknown attribute key `policy`"));
}

#[test]
fn duration_values_parse_from_ms_and_s() {
    let (a, _) = parse_attrs("lead_in=250ms tail=1s", SEGMENT_KEYS, SPAN);
    assert_eq!(a.get_ms("lead_in"), Some(Ok(250)));
    assert_eq!(a.get_ms("tail"), Some(Ok(1000)));
}

#[test]
fn malformed_duration_reports_an_error_value() {
    let (a, _) = parse_attrs("lead_in=soon", SEGMENT_KEYS, SPAN);
    assert!(a.get_ms("lead_in").unwrap().is_err());
}

#[test]
fn float_values_parse() {
    let (a, _) = parse_attrs("max_stretch=2.5", BLOCK_KEYS, SPAN);
    assert_eq!(a.get_f64("max_stretch"), Some(Ok(2.5)));
}

#[test]
fn bare_token_without_equals_is_an_error() {
    let (_, d) = parse_attrs("policy", BLOCK_KEYS, SPAN);
    assert!(d[0].message.contains("expected `key=value`"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p teleprompt-core --test attrs`
Expected: FAIL — `unresolved import teleprompt_core::attrs`.

- [ ] **Step 3: Implement the attribute parser**

```rust
// crates/teleprompt-core/src/attrs.rs
use std::collections::BTreeMap;

use crate::{Diagnostic, SourceSpan};

pub const SEGMENT_KEYS: &[&str] = &[
    "voice.source", "voice.backend", "voice.voice", "voice.speed", "lead_in", "tail", "lang",
];

pub const BLOCK_KEYS: &[&str] = &[
    "scene", "include", "policy", "align", "id", "max_speedup", "max_stretch", "min_stretch",
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attributes(BTreeMap<String, String>);

impl Attributes {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    pub fn get_f64(&self, key: &str) -> Option<Result<f64, String>> {
        self.get(key)
            .map(|v| v.parse::<f64>().map_err(|_| format!("`{v}` is not a number")))
    }

    /// Parses `250ms`, `1s`, `1.5s`, or a bare integer treated as milliseconds.
    pub fn get_ms(&self, key: &str) -> Option<Result<u64, String>> {
        self.get(key).map(parse_duration_ms)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

pub fn parse_duration_ms(v: &str) -> Result<u64, String> {
    let err = || format!("`{v}` is not a duration (try `250ms` or `1s`)");
    if let Some(n) = v.strip_suffix("ms") {
        return n.trim().parse::<u64>().map_err(|_| err());
    }
    if let Some(n) = v.strip_suffix('s') {
        let secs: f64 = n.trim().parse().map_err(|_| err())?;
        if secs < 0.0 {
            return Err(err());
        }
        return Ok((secs * 1000.0).round() as u64);
    }
    v.parse::<u64>().map_err(|_| err())
}

pub fn parse_attrs(
    raw: &str,
    allowed: &[&str],
    span: SourceSpan,
) -> (Attributes, Vec<Diagnostic>) {
    let mut map = BTreeMap::new();
    let mut diags = Vec::new();

    for token in tokenize(raw) {
        if token.starts_with('#') {
            continue; // the id anchor, handled by ident.rs
        }
        let Some((key, value)) = token.split_once('=') else {
            diags.push(
                Diagnostic::error(format!("expected `key=value`, found `{token}`")).at(span),
            );
            continue;
        };
        let key = key.trim();
        if !allowed.contains(&key) {
            let mut d = Diagnostic::error(format!("unknown attribute key `{key}`")).at(span);
            if let Some(sug) = nearest(key, allowed) {
                d = d.with_help(format!("did you mean `{sug}`?"));
            }
            diags.push(d);
            continue;
        }
        let value = value.trim().trim_matches('"').to_string();
        map.insert(key.to_string(), value);
    }

    (Attributes(map), diags)
}

/// Splits on whitespace, keeping double-quoted runs together.
fn tokenize(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for c in raw.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                cur.push(c);
            }
            c if c.is_whitespace() && !in_quotes => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Levenshtein distance, returning the closest allowed key within distance 2.
fn nearest<'a>(key: &str, allowed: &[&'a str]) -> Option<&'a str> {
    allowed
        .iter()
        .map(|c| (*c, levenshtein(key, c)))
        .filter(|(_, d)| *d <= 2)
        .min_by_key(|(_, d)| *d)
        .map(|(c, _)| c)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b_chars: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b_chars.len()).collect();
    let mut cur = vec![0usize; b_chars.len() + 1];

    for (i, ac) in a.chars().enumerate() {
        cur[0] = i + 1;
        for (j, bc) in b_chars.iter().enumerate() {
            let cost = usize::from(ac != *bc);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b_chars.len()]
}
```

```rust
// crates/teleprompt-core/src/lib.rs — add
pub mod attrs;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p teleprompt-core`
Expected: PASS, 33 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/teleprompt-core
git commit -m "feat(core): attribute grammar with key validation and suggestions"
```

---

### Task 5: Configuration types and merge

**Files:**
- Create: `crates/teleprompt-core/src/config.rs`
- Modify: `crates/teleprompt-core/src/lib.rs`
- Test: `crates/teleprompt-core/tests/config.rs`

**Interfaces:**
- Consumes: `Attributes` from Task 4.
- Produces: `Config` (fully resolved, no options) and `PartialConfig` (all fields optional, `serde` Deserialize); `PartialConfig::from_toml(&str)`, `PartialConfig::from_yaml(&str)`, `PartialConfig::from_attrs(&Attributes)`; `Config::default()`; `Config::merged(layers: &[PartialConfig]) -> Config`.

Merge order per spec §3.5: defaults → `teleprompt.toml` → script front matter → chapter front matter → segment/block attributes → CLI flags. Later layers win field by field.

```rust
pub struct Config {
    pub locales: Locales,           // { source: String, targets: Vec<String> }
    pub voice: VoiceConfig,         // { source, backend, voice, speed }
    pub timing: TimingConfig,       // { lead_in_ms, tail_ms, max_stretch, min_stretch, max_speedup }
    pub transition: TransitionConfig, // { kind, duration: Auto|Fixed(u64), min_ms, max_ms }
    pub scenes: BTreeMap<String, SceneConfig>, // scene name -> { adapter, settings }
}
```

- [ ] **Step 1: Write the failing tests**

```rust
// crates/teleprompt-core/tests/config.rs
use teleprompt_core::attrs::{parse_attrs, SEGMENT_KEYS};
use teleprompt_core::config::{Config, PartialConfig, TransitionDuration};
use teleprompt_core::SourceSpan;

#[test]
fn defaults_match_the_spec() {
    let c = Config::default();
    assert_eq!(c.timing.lead_in_ms, 150);
    assert_eq!(c.timing.tail_ms, 150);
    assert_eq!(c.timing.max_stretch, 3.0);
    assert_eq!(c.timing.min_stretch, 0.33);
    assert_eq!(c.timing.max_speedup, 2.0);
    assert_eq!(c.transition.max_ms, 600);
    assert_eq!(c.transition.min_ms, 0);
    assert_eq!(c.transition.duration, TransitionDuration::Auto);
    assert_eq!(c.locales.source, "en");
}

#[test]
fn later_layers_override_earlier_ones_field_by_field() {
    let project = PartialConfig::from_toml("[timing]\nlead_in_ms = 300\ntail_ms = 400\n").unwrap();
    let script = PartialConfig::from_yaml("timing:\n  tail_ms: 500\n").unwrap();
    let c = Config::merged(&[project, script]);
    assert_eq!(c.timing.lead_in_ms, 300, "untouched field survives");
    assert_eq!(c.timing.tail_ms, 500, "later layer wins");
}

#[test]
fn absent_layers_change_nothing() {
    let c = Config::merged(&[PartialConfig::default(), PartialConfig::default()]);
    assert_eq!(c, Config::default());
}

#[test]
fn attributes_become_a_config_layer() {
    let span = SourceSpan { line: 1, column: 1, len: 0 };
    let (a, _) = parse_attrs("lead_in=400ms voice.source=cloned", SEGMENT_KEYS, span);
    let c = Config::merged(&[PartialConfig::from_attrs(&a)]);
    assert_eq!(c.timing.lead_in_ms, 400);
    assert_eq!(c.voice.source, "cloned");
}

#[test]
fn scene_config_carries_an_adapter_and_free_form_settings() {
    let yaml = "scene:\n  browser:\n    adapter: playwright\n    base_url: http://localhost:3000\n";
    let c = Config::merged(&[PartialConfig::from_yaml(yaml).unwrap()]);
    let s = c.scenes.get("browser").unwrap();
    assert_eq!(s.adapter, "playwright");
    assert_eq!(s.settings.get("base_url").unwrap(), "http://localhost:3000");
}

#[test]
fn scene_adapter_defaults_by_scene_name() {
    let c = Config::merged(&[PartialConfig::from_yaml("scene:\n  terminal: {}\n").unwrap()]);
    assert_eq!(c.scenes.get("terminal").unwrap().adapter, "vhs");
}

#[test]
fn malformed_yaml_is_reported_not_panicked() {
    assert!(PartialConfig::from_yaml("timing:\n  lead_in_ms: [nope\n").is_err());
}

#[test]
fn transition_duration_accepts_auto_or_a_number() {
    let auto = PartialConfig::from_yaml("transition:\n  duration: auto\n").unwrap();
    assert_eq!(Config::merged(&[auto]).transition.duration, TransitionDuration::Auto);

    let fixed = PartialConfig::from_yaml("transition:\n  duration: 250ms\n").unwrap();
    assert_eq!(Config::merged(&[fixed]).transition.duration, TransitionDuration::Fixed(250));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p teleprompt-core --test config`
Expected: FAIL — `unresolved import teleprompt_core::config`.

- [ ] **Step 3: Implement the config types**

```rust
// crates/teleprompt-core/src/config.rs
use std::collections::BTreeMap;

use serde::Deserialize;

use crate::attrs::{parse_duration_ms, Attributes};

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub locales: Locales,
    pub voice: VoiceConfig,
    pub timing: TimingConfig,
    pub transition: TransitionConfig,
    pub scenes: BTreeMap<String, SceneConfig>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Locales {
    pub source: String,
    pub targets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VoiceConfig {
    pub source: String,
    pub backend: String,
    pub voice: Option<String>,
    pub speed: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimingConfig {
    pub lead_in_ms: u64,
    pub tail_ms: u64,
    pub max_stretch: f64,
    pub min_stretch: f64,
    pub max_speedup: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionDuration {
    Auto,
    Fixed(u64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransitionConfig {
    pub kind: String,
    pub duration: TransitionDuration,
    pub min_ms: u64,
    pub max_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneConfig {
    pub adapter: String,
    pub settings: BTreeMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            locales: Locales { source: "en".into(), targets: Vec::new() },
            voice: VoiceConfig {
                source: "synthetic".into(),
                backend: "null".into(),
                voice: None,
                speed: 1.0,
            },
            timing: TimingConfig {
                lead_in_ms: 150,
                tail_ms: 150,
                max_stretch: 3.0,
                min_stretch: 0.33,
                max_speedup: 2.0,
            },
            transition: TransitionConfig {
                kind: "crossfade".into(),
                duration: TransitionDuration::Auto,
                min_ms: 0,
                max_ms: 600,
            },
            scenes: BTreeMap::new(),
        }
    }
}

/// Default adapter for a scene name, used when config omits it.
pub fn default_adapter(scene: &str) -> &'static str {
    match scene {
        "browser" => "playwright",
        "terminal" => "vhs",
        "media" => "media",
        _ => "mock",
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartialConfig {
    pub locales: Option<PartialLocales>,
    pub voice: Option<PartialVoice>,
    pub timing: Option<PartialTiming>,
    pub transition: Option<PartialTransition>,
    pub scene: Option<BTreeMap<String, PartialScene>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartialLocales {
    pub source: Option<String>,
    pub targets: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartialVoice {
    pub source: Option<String>,
    pub backend: Option<String>,
    pub voice: Option<String>,
    pub speed: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartialTiming {
    pub lead_in_ms: Option<u64>,
    pub tail_ms: Option<u64>,
    pub max_stretch: Option<f64>,
    pub min_stretch: Option<f64>,
    pub max_speedup: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartialTransition {
    pub kind: Option<String>,
    pub duration: Option<String>,
    pub min_ms: Option<u64>,
    pub max_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PartialScene {
    pub adapter: Option<String>,
    #[serde(flatten)]
    pub settings: BTreeMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("invalid YAML: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("invalid TOML: {0}")]
    Toml(#[from] toml::de::Error),
}

impl PartialConfig {
    pub fn from_yaml(s: &str) -> Result<Self, ConfigError> {
        if s.trim().is_empty() {
            return Ok(Self::default());
        }
        Ok(serde_yaml::from_str(s)?)
    }

    pub fn from_toml(s: &str) -> Result<Self, ConfigError> {
        Ok(toml::from_str(s)?)
    }

    /// Builds a layer from segment or block attributes. Unparseable values are
    /// dropped here; `parse_attrs` has already reported them as diagnostics.
    pub fn from_attrs(a: &Attributes) -> Self {
        let mut c = Self::default();
        let mut timing = PartialTiming::default();
        let mut voice = PartialVoice::default();

        if let Some(Ok(ms)) = a.get_ms("lead_in") {
            timing.lead_in_ms = Some(ms);
        }
        if let Some(Ok(ms)) = a.get_ms("tail") {
            timing.tail_ms = Some(ms);
        }
        if let Some(Ok(v)) = a.get_f64("max_stretch") {
            timing.max_stretch = Some(v);
        }
        if let Some(Ok(v)) = a.get_f64("min_stretch") {
            timing.min_stretch = Some(v);
        }
        if let Some(Ok(v)) = a.get_f64("max_speedup") {
            timing.max_speedup = Some(v);
        }
        voice.source = a.get("voice.source").map(str::to_string);
        voice.backend = a.get("voice.backend").map(str::to_string);
        voice.voice = a.get("voice.voice").map(str::to_string);
        if let Some(Ok(v)) = a.get_f64("voice.speed") {
            voice.speed = Some(v);
        }

        c.timing = Some(timing);
        c.voice = Some(voice);
        c
    }
}

macro_rules! set {
    ($target:expr, $src:expr) => {
        if let Some(v) = $src {
            $target = v;
        }
    };
}

impl Config {
    pub fn merged(layers: &[PartialConfig]) -> Self {
        let mut c = Config::default();
        for layer in layers {
            if let Some(l) = &layer.locales {
                set!(c.locales.source, l.source.clone());
                set!(c.locales.targets, l.targets.clone());
            }
            if let Some(v) = &layer.voice {
                set!(c.voice.source, v.source.clone());
                set!(c.voice.backend, v.backend.clone());
                if v.voice.is_some() {
                    c.voice.voice = v.voice.clone();
                }
                set!(c.voice.speed, v.speed);
            }
            if let Some(t) = &layer.timing {
                set!(c.timing.lead_in_ms, t.lead_in_ms);
                set!(c.timing.tail_ms, t.tail_ms);
                set!(c.timing.max_stretch, t.max_stretch);
                set!(c.timing.min_stretch, t.min_stretch);
                set!(c.timing.max_speedup, t.max_speedup);
            }
            if let Some(t) = &layer.transition {
                set!(c.transition.kind, t.kind.clone());
                set!(c.transition.min_ms, t.min_ms);
                set!(c.transition.max_ms, t.max_ms);
                if let Some(d) = &t.duration {
                    c.transition.duration = if d == "auto" {
                        TransitionDuration::Auto
                    } else {
                        match parse_duration_ms(d) {
                            Ok(ms) => TransitionDuration::Fixed(ms),
                            Err(_) => TransitionDuration::Auto,
                        }
                    };
                }
            }
            if let Some(scenes) = &layer.scene {
                for (name, ps) in scenes {
                    let entry = c.scenes.entry(name.clone()).or_insert_with(|| SceneConfig {
                        adapter: default_adapter(name).to_string(),
                        settings: BTreeMap::new(),
                    });
                    set!(entry.adapter, ps.adapter.clone());
                    for (k, v) in &ps.settings {
                        entry.settings.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        c
    }
}
```

```rust
// crates/teleprompt-core/src/lib.rs — add
pub mod config;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p teleprompt-core`
Expected: PASS, 41 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/teleprompt-core
git commit -m "feat(core): configuration types with layered merge"
```

---

### Task 6: Scene contract and mock adapter

**Files:**
- Create: `crates/teleprompt-scene/Cargo.toml`, `crates/teleprompt-scene/src/lib.rs`
- Create: `crates/teleprompt-scene/src/contract.rs`, `src/registry.rs`, `src/mock.rs`
- Test: `crates/teleprompt-scene/tests/mock.rs`

**Interfaces:**
- Consumes: `Diagnostic`, `SourceSpan`, `Hash` from Task 1; `SceneConfig` from Task 5.
- Produces:

```rust
pub struct BlockSource { pub scene: String, pub body: String, pub span: SourceSpan }
pub struct Validated { pub scene: String, pub body: String }
pub struct Span { pub id: String, pub source: String, pub hash: Hash, pub index: usize }
pub enum Measured { Exact(u64), Estimated(u64), Unknown }   // milliseconds

pub trait SceneCompiler: Send + Sync {
    fn kind(&self) -> &'static str;
    fn validate(&self, src: &BlockSource) -> Result<Validated, Vec<Diagnostic>>;
    fn spans(&self, v: &Validated, block_id: &str) -> Result<Vec<Span>, Vec<Diagnostic>>;
    fn estimate(&self, span: &Span) -> Measured;
}

pub struct SceneRegistry;
impl SceneRegistry {
    pub fn with_builtins() -> Self;
    pub fn register(&mut self, adapter: Box<dyn SceneCompiler>);
    pub fn get(&self, adapter: &str) -> Option<&dyn SceneCompiler>;
}
```

The `mock` adapter's language: one directive per line, `wait <duration>` and `mark`. Blank lines and `#` comments ignored. Everything else is an error. Deterministic durations make it the scheduler's test fixture.

- [ ] **Step 1: Write the failing tests**

```rust
// crates/teleprompt-scene/tests/mock.rs
use teleprompt_core::SourceSpan;
use teleprompt_scene::{BlockSource, Measured, MockScene, SceneCompiler, SceneRegistry};

const SPAN: SourceSpan = SourceSpan { line: 1, column: 1, len: 0 };

fn src(body: &str) -> BlockSource {
    BlockSource { scene: "mock".into(), body: body.into(), span: SPAN }
}

#[test]
fn valid_body_passes_validation() {
    let m = MockScene;
    assert!(m.validate(&src("wait 500ms\nmark\nwait 1s\n")).is_ok());
}

#[test]
fn unknown_directive_is_an_error_naming_the_line() {
    let m = MockScene;
    let e = m.validate(&src("wait 500ms\nclick everything\n")).unwrap_err();
    assert_eq!(e.len(), 1);
    assert!(e[0].message.contains("unknown mock directive `click`"));
    assert_eq!(e[0].span.unwrap().line, 2);
}

#[test]
fn comments_and_blank_lines_are_ignored() {
    let m = MockScene;
    assert!(m.validate(&src("# setup\n\nwait 100ms\n")).is_ok());
}

#[test]
fn marks_split_the_block_into_spans() {
    let m = MockScene;
    let v = m.validate(&src("wait 500ms\nmark\nwait 300ms\n")).unwrap();
    let spans = m.spans(&v, "a-1-a").unwrap();
    assert_eq!(spans.len(), 2);
    assert_eq!(spans[0].id, "a-1-a#0");
    assert_eq!(spans[1].id, "a-1-a#1");
}

#[test]
fn a_block_with_no_marks_is_one_span() {
    let m = MockScene;
    let v = m.validate(&src("wait 500ms\nwait 300ms\n")).unwrap();
    assert_eq!(m.spans(&v, "b").unwrap().len(), 1);
}

#[test]
fn estimate_sums_the_waits_exactly() {
    let m = MockScene;
    let v = m.validate(&src("wait 500ms\nwait 1s\n")).unwrap();
    let spans = m.spans(&v, "b").unwrap();
    assert_eq!(m.estimate(&spans[0]), Measured::Exact(1500));
}

#[test]
fn span_hashes_differ_by_content_and_repeat_for_identical_content() {
    let m = MockScene;
    let v1 = m.validate(&src("wait 500ms\nmark\nwait 500ms\n")).unwrap();
    let s1 = m.spans(&v1, "b").unwrap();
    assert_eq!(s1[0].hash, s1[1].hash, "identical spans hash identically");

    let v2 = m.validate(&src("wait 501ms\n")).unwrap();
    let s2 = m.spans(&v2, "b").unwrap();
    assert_ne!(s1[0].hash, s2[0].hash);
}

#[test]
fn registry_resolves_builtin_adapters_and_rejects_unknown_ones() {
    let r = SceneRegistry::with_builtins();
    assert_eq!(r.get("mock").map(|a| a.kind()), Some("mock"));
    assert!(r.get("playwright").is_none(), "not available in M0");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p teleprompt-scene`
Expected: FAIL — crate does not exist.

- [ ] **Step 3: Create the crate and contract**

```toml
# crates/teleprompt-scene/Cargo.toml
[package]
name = "teleprompt-scene"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
teleprompt-core = { path = "../teleprompt-core" }
serde.workspace = true
```

```rust
// crates/teleprompt-scene/src/contract.rs
use teleprompt_core::{Diagnostic, Hash, SourceSpan};

#[derive(Debug, Clone)]
pub struct BlockSource {
    pub scene: String,
    pub body: String,
    pub span: SourceSpan,
}

#[derive(Debug, Clone)]
pub struct Validated {
    pub scene: String,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct Span {
    pub id: String,
    pub source: String,
    pub hash: Hash,
    pub index: usize,
}

/// Duration of a span, in milliseconds.
///
/// `Exact` comes from a declarative language that states its own timing.
/// `Estimated` is a guess the compiler may improve by measuring.
/// `Unknown` means the adapter cannot say and a measuring pass is required
/// (M1 and later; M0 adapters never return it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Measured {
    Exact(u64),
    Estimated(u64),
    Unknown,
}

impl Measured {
    pub fn duration_ms(&self) -> Option<u64> {
        match self {
            Measured::Exact(ms) | Measured::Estimated(ms) => Some(*ms),
            Measured::Unknown => None,
        }
    }

    pub fn source_label(&self) -> &'static str {
        match self {
            Measured::Exact(_) => "exact",
            Measured::Estimated(_) => "estimated",
            Measured::Unknown => "unknown",
        }
    }
}

pub trait SceneCompiler: Send + Sync {
    fn kind(&self) -> &'static str;
    fn validate(&self, src: &BlockSource) -> Result<Validated, Vec<Diagnostic>>;
    fn spans(&self, v: &Validated, block_id: &str) -> Result<Vec<Span>, Vec<Diagnostic>>;
    fn estimate(&self, span: &Span) -> Measured;
}
```

- [ ] **Step 4: Implement the mock adapter and registry**

```rust
// crates/teleprompt-scene/src/mock.rs
use teleprompt_core::attrs::parse_duration_ms;
use teleprompt_core::{Diagnostic, Hash, SourceSpan};

use crate::contract::{BlockSource, Measured, SceneCompiler, Span, Validated};

pub struct MockScene;

impl SceneCompiler for MockScene {
    fn kind(&self) -> &'static str {
        "mock"
    }

    fn validate(&self, src: &BlockSource) -> Result<Validated, Vec<Diagnostic>> {
        let mut diags = Vec::new();
        for (i, line) in src.body.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let span = SourceSpan { line: src.span.line + i + 1, column: 1, len: line.len() };
            let mut parts = line.split_whitespace();
            match parts.next() {
                Some("mark") => {}
                Some("wait") => match parts.next() {
                    Some(v) => {
                        if let Err(msg) = parse_duration_ms(v) {
                            diags.push(Diagnostic::error(msg).at(span));
                        }
                    }
                    None => diags.push(
                        Diagnostic::error("`wait` needs a duration").at(span).with_help("e.g. `wait 500ms`"),
                    ),
                },
                Some(other) => diags.push(
                    Diagnostic::error(format!("unknown mock directive `{other}`"))
                        .at(span)
                        .with_help("mock understands `wait <duration>` and `mark`"),
                ),
                None => {}
            }
        }

        if diags.is_empty() {
            Ok(Validated { scene: src.scene.clone(), body: src.body.clone() })
        } else {
            Err(diags)
        }
    }

    fn spans(&self, v: &Validated, block_id: &str) -> Result<Vec<Span>, Vec<Diagnostic>> {
        let chunks: Vec<String> = v
            .body
            .lines()
            .collect::<Vec<_>>()
            .split(|l| l.trim() == "mark")
            .map(|lines| lines.join("\n"))
            .collect();

        Ok(chunks
            .into_iter()
            .enumerate()
            .map(|(index, source)| Span {
                id: format!("{block_id}#{index}"),
                hash: Hash::of(source.trim().as_bytes()),
                source,
                index,
            })
            .collect())
    }

    fn estimate(&self, span: &Span) -> Measured {
        let total: u64 = span
            .source
            .lines()
            .filter_map(|l| l.trim().strip_prefix("wait "))
            .filter_map(|v| parse_duration_ms(v.trim()).ok())
            .sum();
        Measured::Exact(total)
    }
}
```

```rust
// crates/teleprompt-scene/src/registry.rs
use std::collections::BTreeMap;

use crate::contract::SceneCompiler;
use crate::mock::MockScene;

#[derive(Default)]
pub struct SceneRegistry {
    adapters: BTreeMap<&'static str, Box<dyn SceneCompiler>>,
}

impl SceneRegistry {
    pub fn with_builtins() -> Self {
        let mut r = Self::default();
        r.register(Box::new(MockScene));
        r
    }

    pub fn register(&mut self, adapter: Box<dyn SceneCompiler>) {
        self.adapters.insert(adapter.kind(), adapter);
    }

    pub fn get(&self, adapter: &str) -> Option<&dyn SceneCompiler> {
        self.adapters.get(adapter).map(AsRef::as_ref)
    }

    pub fn available(&self) -> Vec<&'static str> {
        self.adapters.keys().copied().collect()
    }
}
```

```rust
// crates/teleprompt-scene/src/lib.rs
pub mod contract;
pub mod mock;
pub mod registry;

pub use contract::{BlockSource, Measured, SceneCompiler, Span, Validated};
pub use mock::MockScene;
pub use registry::SceneRegistry;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p teleprompt-scene`
Expected: PASS, 8 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/teleprompt-scene
git commit -m "feat(scene): compile-time scene contract with mock adapter"
```

---

### Task 7: Voice contract, fallback ladder, and the null backend

**Files:**
- Create: `crates/teleprompt-voice/Cargo.toml`, `src/lib.rs`, `src/contract.rs`, `src/source.rs`, `src/null.rs`
- Test: `crates/teleprompt-voice/tests/null.rs`, `crates/teleprompt-voice/tests/ladder.rs`

**Interfaces:**
- Consumes: `Hash` from Task 1.
- Produces:

```rust
pub enum VoiceSource { Recorded, Cloned, Synthetic }   // ordered high -> low
impl VoiceSource {
    pub fn parse(s: &str) -> Option<Self>;
    pub fn label(&self) -> &'static str;
    pub fn next_lower(&self) -> Option<Self>;
}

pub struct Resolution { pub requested: VoiceSource, pub actual: VoiceSource, pub downgrade_reason: Option<String> }
pub fn resolve_source(requested: VoiceSource, available: &dyn Fn(VoiceSource) -> Result<(), String>) -> Result<Resolution, String>;

pub struct VoiceCapabilities { pub languages: LanguageSupport, pub cloning: bool, pub cross_lingual: bool, pub word_timings: bool, pub ssml: bool, pub speed_control: bool }
pub enum LanguageSupport { Any, Enumerated(Vec<String>) }

pub struct SynthRequest { pub text: String, pub locale: String, pub voice: Option<String>, pub speed: f64 }
pub struct SynthResult { pub duration_ms: u64, pub audio_hash: Hash, pub word_timings: Option<Vec<WordTiming>> }
pub struct WordTiming { pub word: String, pub start_ms: u64, pub end_ms: u64 }

pub trait VoiceBackend: Send + Sync {
    fn id(&self) -> &'static str;
    fn capabilities(&self) -> VoiceCapabilities;
    fn synthesize(&self, req: &SynthRequest) -> Result<SynthResult, VoiceError>;
    fn cache_key(&self, req: &SynthRequest) -> String;
}

pub struct NullVoice { pub wpm: f64 }   // default 150.0
```

Null duration model: `words / wpm * 60_000`, divided by `speed`, plus punctuation pauses — comma 150 ms, colon/semicolon 250 ms, sentence-ending `.!?` 350 ms. Deterministic and offline.

- [ ] **Step 1: Write the failing tests**

```rust
// crates/teleprompt-voice/tests/null.rs
use teleprompt_voice::{NullVoice, SynthRequest, VoiceBackend};

fn req(text: &str) -> SynthRequest {
    SynthRequest { text: text.into(), locale: "en".into(), voice: None, speed: 1.0 }
}

#[test]
fn duration_scales_with_word_count() {
    let v = NullVoice::default();
    let short = v.synthesize(&req("one two three")).unwrap().duration_ms;
    let long = v.synthesize(&req("one two three four five six")).unwrap().duration_ms;
    assert!(long > short);
}

#[test]
fn six_words_at_150_wpm_is_2400ms_plus_no_punctuation() {
    let v = NullVoice { wpm: 150.0 };
    // 6 words / 150 wpm * 60_000 = 2400 ms
    assert_eq!(v.synthesize(&req("one two three four five six")).unwrap().duration_ms, 2400);
}

#[test]
fn punctuation_adds_pauses() {
    let v = NullVoice { wpm: 150.0 };
    let plain = v.synthesize(&req("one two")).unwrap().duration_ms;
    let stopped = v.synthesize(&req("one two.")).unwrap().duration_ms;
    assert_eq!(stopped - plain, 350);

    let comma = v.synthesize(&req("one, two")).unwrap().duration_ms;
    assert_eq!(comma - plain, 150);
}

#[test]
fn speed_divides_the_duration() {
    let v = NullVoice { wpm: 150.0 };
    let mut r = req("one two three four five six");
    r.speed = 2.0;
    assert_eq!(v.synthesize(&r).unwrap().duration_ms, 1200);
}

#[test]
fn synthesis_is_deterministic() {
    let v = NullVoice::default();
    let a = v.synthesize(&req("Welcome to Acme.")).unwrap();
    let b = v.synthesize(&req("Welcome to Acme.")).unwrap();
    assert_eq!(a.duration_ms, b.duration_ms);
    assert_eq!(a.audio_hash, b.audio_hash);
}

#[test]
fn cache_key_varies_with_every_input_that_changes_the_output() {
    let v = NullVoice::default();
    let base = req("hello");
    let mut other_locale = base.clone();
    other_locale.locale = "nl".into();
    let mut other_speed = base.clone();
    other_speed.speed = 1.5;

    assert_ne!(v.cache_key(&base), v.cache_key(&other_locale));
    assert_ne!(v.cache_key(&base), v.cache_key(&other_speed));
    assert_eq!(v.cache_key(&base), v.cache_key(&req("hello")));
}

#[test]
fn empty_text_is_zero_duration() {
    let v = NullVoice::default();
    assert_eq!(v.synthesize(&req("   ")).unwrap().duration_ms, 0);
}

#[test]
fn null_backend_declares_honest_capabilities() {
    let c = NullVoice::default().capabilities();
    assert!(!c.cloning);
    assert!(!c.word_timings);
    assert!(c.speed_control);
}
```

```rust
// crates/teleprompt-voice/tests/ladder.rs
use teleprompt_voice::{resolve_source, VoiceSource};

#[test]
fn available_tier_is_used_unchanged() {
    let r = resolve_source(VoiceSource::Recorded, &|_| Ok(())).unwrap();
    assert_eq!(r.actual, VoiceSource::Recorded);
    assert!(r.downgrade_reason.is_none());
}

#[test]
fn unavailable_tier_drops_one_step_and_records_why() {
    let r = resolve_source(VoiceSource::Recorded, &|t| match t {
        VoiceSource::Recorded => Err("take stale".into()),
        _ => Ok(()),
    })
    .unwrap();
    assert_eq!(r.requested, VoiceSource::Recorded);
    assert_eq!(r.actual, VoiceSource::Cloned);
    assert_eq!(r.downgrade_reason.as_deref(), Some("take stale"));
}

#[test]
fn the_ladder_descends_more_than_one_rung_when_needed() {
    let r = resolve_source(VoiceSource::Recorded, &|t| match t {
        VoiceSource::Synthetic => Ok(()),
        _ => Err("unavailable".into()),
    })
    .unwrap();
    assert_eq!(r.actual, VoiceSource::Synthetic);
}

#[test]
fn exhausting_the_ladder_is_an_error() {
    let e = resolve_source(VoiceSource::Recorded, &|_| Err("nope".into())).unwrap_err();
    assert!(e.contains("no voice source available"));
}

#[test]
fn a_lower_request_never_climbs_back_up() {
    let r = resolve_source(VoiceSource::Synthetic, &|_| Ok(())).unwrap();
    assert_eq!(r.actual, VoiceSource::Synthetic);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p teleprompt-voice`
Expected: FAIL — crate does not exist.

- [ ] **Step 3: Create the crate and contract**

```toml
# crates/teleprompt-voice/Cargo.toml
[package]
name = "teleprompt-voice"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
teleprompt-core = { path = "../teleprompt-core" }
serde.workspace = true
thiserror.workspace = true
```

```rust
// crates/teleprompt-voice/src/contract.rs
use teleprompt_core::Hash;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanguageSupport {
    Any,
    Enumerated(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceCapabilities {
    pub languages: LanguageSupport,
    pub cloning: bool,
    pub cross_lingual: bool,
    pub word_timings: bool,
    pub ssml: bool,
    pub speed_control: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SynthRequest {
    pub text: String,
    pub locale: String,
    pub voice: Option<String>,
    pub speed: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordTiming {
    pub word: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Debug, Clone)]
pub struct SynthResult {
    pub duration_ms: u64,
    pub audio_hash: Hash,
    pub word_timings: Option<Vec<WordTiming>>,
}

#[derive(Debug, thiserror::Error)]
pub enum VoiceError {
    #[error("backend `{backend}` does not support {what}")]
    Unsupported { backend: &'static str, what: String },
    #[error("{0}")]
    Other(String),
}

pub trait VoiceBackend: Send + Sync {
    fn id(&self) -> &'static str;
    fn capabilities(&self) -> VoiceCapabilities;
    fn synthesize(&self, req: &SynthRequest) -> Result<SynthResult, VoiceError>;
    fn cache_key(&self, req: &SynthRequest) -> String;
}
```

- [ ] **Step 4: Implement the fallback ladder**

```rust
// crates/teleprompt-voice/src/source.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceSource {
    Recorded,
    Cloned,
    Synthetic,
}

impl VoiceSource {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "recorded" => Some(Self::Recorded),
            "cloned" => Some(Self::Cloned),
            "synthetic" => Some(Self::Synthetic),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Recorded => "recorded",
            Self::Cloned => "cloned",
            Self::Synthetic => "synthetic",
        }
    }

    pub fn next_lower(&self) -> Option<Self> {
        match self {
            Self::Recorded => Some(Self::Cloned),
            Self::Cloned => Some(Self::Synthetic),
            Self::Synthetic => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub requested: VoiceSource,
    pub actual: VoiceSource,
    pub downgrade_reason: Option<String>,
}

/// Walks the ladder `recorded -> cloned -> synthetic`, stopping at the first
/// tier `available` accepts. The reason recorded is the *first* rejection, not
/// the last, so the message explains why the author's own voice was not used.
pub fn resolve_source(
    requested: VoiceSource,
    available: &dyn Fn(VoiceSource) -> Result<(), String>,
) -> Result<Resolution, String> {
    let mut tier = requested;
    let mut reason: Option<String> = None;

    loop {
        match available(tier) {
            Ok(()) => {
                return Ok(Resolution { requested, actual: tier, downgrade_reason: reason });
            }
            Err(why) => {
                if reason.is_none() {
                    reason = Some(why);
                }
                match tier.next_lower() {
                    Some(next) => tier = next,
                    None => {
                        return Err(format!(
                            "no voice source available (started at `{}`): {}",
                            requested.label(),
                            reason.unwrap_or_default()
                        ))
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 5: Implement the null backend**

```rust
// crates/teleprompt-voice/src/null.rs
use teleprompt_core::Hash;

use crate::contract::{
    LanguageSupport, SynthRequest, SynthResult, VoiceBackend, VoiceCapabilities, VoiceError,
};

pub struct NullVoice {
    pub wpm: f64,
}

impl Default for NullVoice {
    fn default() -> Self {
        Self { wpm: 150.0 }
    }
}

const COMMA_MS: u64 = 150;
const CLAUSE_MS: u64 = 250;
const SENTENCE_MS: u64 = 350;

pub fn estimate_ms(text: &str, wpm: f64, speed: f64) -> u64 {
    let words = text.split_whitespace().count() as f64;
    if words == 0.0 {
        return 0;
    }
    let speech = words / wpm * 60_000.0;
    let pauses: u64 = text
        .chars()
        .map(|c| match c {
            ',' => COMMA_MS,
            ':' | ';' => CLAUSE_MS,
            '.' | '!' | '?' => SENTENCE_MS,
            _ => 0,
        })
        .sum();
    ((speech + pauses as f64) / speed).round() as u64
}

impl VoiceBackend for NullVoice {
    fn id(&self) -> &'static str {
        "null"
    }

    fn capabilities(&self) -> VoiceCapabilities {
        VoiceCapabilities {
            languages: LanguageSupport::Any,
            cloning: false,
            cross_lingual: false,
            word_timings: false,
            ssml: false,
            speed_control: true,
        }
    }

    fn synthesize(&self, req: &SynthRequest) -> Result<SynthResult, VoiceError> {
        if req.speed <= 0.0 {
            return Err(VoiceError::Other("speed must be greater than zero".into()));
        }
        Ok(SynthResult {
            duration_ms: estimate_ms(&req.text, self.wpm, req.speed),
            audio_hash: Hash::of(self.cache_key(req).as_bytes()),
            word_timings: None,
        })
    }

    fn cache_key(&self, req: &SynthRequest) -> String {
        format!(
            "null/{}/{}/{}/{}/{}",
            env!("CARGO_PKG_VERSION"),
            req.locale,
            req.voice.as_deref().unwrap_or("-"),
            req.speed,
            Hash::of(req.text.as_bytes())
        )
    }
}
```

```rust
// crates/teleprompt-voice/src/lib.rs
pub mod contract;
pub mod null;
pub mod source;

pub use contract::{
    LanguageSupport, SynthRequest, SynthResult, VoiceBackend, VoiceCapabilities, VoiceError,
    WordTiming,
};
pub use null::{estimate_ms, NullVoice};
pub use source::{resolve_source, Resolution, VoiceSource};
```

Note: `SynthRequest` needs `#[derive(Clone)]` for the cache-key test, which is already in the struct definition above.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p teleprompt-voice`
Expected: PASS, 13 tests.

- [ ] **Step 7: Commit**

```bash
git add crates/teleprompt-voice
git commit -m "feat(voice): backend contract, fallback ladder, and null backend"
```

---

### Task 8: Program resolution

**Files:**
- Create: `crates/teleprompt-core/src/program.rs`
- Modify: `crates/teleprompt-core/src/lib.rs`, `crates/teleprompt-core/Cargo.toml`
- Test: `crates/teleprompt-core/tests/program.rs`

**Interfaces:**
- Consumes: `Script`/`Node` (Task 2), `assign_ids` (Task 3), `parse_attrs` (Task 4), `Config`/`PartialConfig` (Task 5).
- Produces:

```rust
pub struct Program {
    pub script_name: String,
    pub locale: String,
    pub config: Config,
    pub items: Vec<Item>,
}
pub enum Item {
    Narration { id: String, text: String, source_hash: Hash, config: Config },
    Action { block_id: String, scene: String, body: String, config: Config, policy: String, align: String },
    Pause { ms: u64 },
}
pub fn resolve(script: &Script, script_name: &str, locale: &str, project: &PartialConfig, cli: &PartialConfig) -> Result<Program, Diagnostics>;
```

`resolve` flattens chapters into a single ordered item list, applies the six-level merge per item, and computes `source_hash` over the normalised segment text.

- [ ] **Step 1: Write the failing tests**

```rust
// crates/teleprompt-core/tests/program.rs
use teleprompt_core::config::PartialConfig;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::{resolve, Item};
use teleprompt_core::Hash;

const SRC: &str = r#"---
timing:
  lead_in_ms: 200
scene:
  mock: { adapter: mock }
---

# Intro

Welcome. {#welcome}

```teleprompt scene=mock policy=concurrent
wait 500ms
```

Second. {#second lead_in=400ms}

<!-- teleprompt: pause 800ms -->
"#;

fn program() -> teleprompt_core::program::Program {
    let mut s = parse_script(SRC).unwrap();
    teleprompt_core::ident::assign_ids(&mut s);
    resolve(&s, "demo.md", "en", &PartialConfig::default(), &PartialConfig::default()).unwrap()
}

#[test]
fn chapters_flatten_into_one_ordered_item_list() {
    let p = program();
    let kinds: Vec<&str> = p
        .items
        .iter()
        .map(|i| match i {
            Item::Narration { .. } => "narration",
            Item::Action { .. } => "action",
            Item::Pause { .. } => "pause",
        })
        .collect();
    assert_eq!(kinds, ["narration", "action", "narration", "pause"]);
}

#[test]
fn front_matter_config_reaches_every_item() {
    let p = program();
    let Item::Narration { config, .. } = &p.items[0] else { panic!() };
    assert_eq!(config.timing.lead_in_ms, 200);
}

#[test]
fn segment_attributes_override_front_matter_for_that_segment_only() {
    let p = program();
    let Item::Narration { config: first, .. } = &p.items[0] else { panic!() };
    let Item::Narration { config: second, .. } = &p.items[2] else { panic!() };
    assert_eq!(first.timing.lead_in_ms, 200);
    assert_eq!(second.timing.lead_in_ms, 400);
}

#[test]
fn cli_flags_beat_everything() {
    let mut s = parse_script(SRC).unwrap();
    teleprompt_core::ident::assign_ids(&mut s);
    let cli = PartialConfig::from_yaml("timing:\n  lead_in_ms: 999\n").unwrap();
    let p = resolve(&s, "demo.md", "en", &PartialConfig::default(), &cli).unwrap();
    let Item::Narration { config, .. } = &p.items[2] else { panic!() };
    assert_eq!(config.timing.lead_in_ms, 999, "CLI outranks the segment attribute");
}

#[test]
fn source_hash_covers_the_normalised_text_only() {
    let p = program();
    let Item::Narration { source_hash, text, .. } = &p.items[0] else { panic!() };
    assert_eq!(*source_hash, Hash::of(text.as_bytes()));
}

#[test]
fn block_policy_and_align_default_when_unset() {
    let mut s = parse_script("# A\n\nOne.\n\n```teleprompt scene=mock\nwait 1s\n```\n").unwrap();
    teleprompt_core::ident::assign_ids(&mut s);
    let p = resolve(&s, "d.md", "en", &PartialConfig::default(), &PartialConfig::default()).unwrap();
    let Item::Action { policy, align, .. } = &p.items[1] else { panic!() };
    assert_eq!(policy, "hold");
    assert_eq!(align, "start");
}

#[test]
fn an_action_block_without_a_scene_is_an_error() {
    let mut s = parse_script("# A\n\nOne.\n\n```teleprompt\nwait 1s\n```\n").unwrap();
    teleprompt_core::ident::assign_ids(&mut s);
    let e = resolve(&s, "d.md", "en", &PartialConfig::default(), &PartialConfig::default())
        .unwrap_err();
    assert!(e.0[0].message.contains("action block has no `scene`"));
}

#[test]
fn unknown_attribute_keys_surface_as_errors() {
    let mut s = parse_script("# A\n\nOne. {#a polcy=hold}\n").unwrap();
    teleprompt_core::ident::assign_ids(&mut s);
    let e = resolve(&s, "d.md", "en", &PartialConfig::default(), &PartialConfig::default())
        .unwrap_err();
    assert!(e.0[0].message.contains("unknown attribute key `polcy`"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p teleprompt-core --test program`
Expected: FAIL — `unresolved import teleprompt_core::program`.

- [ ] **Step 3: Implement resolution**

```rust
// crates/teleprompt-core/src/program.rs
use crate::ast::{Directive, Node, Script};
use crate::attrs::{parse_attrs, BLOCK_KEYS, SEGMENT_KEYS};
use crate::config::{Config, PartialConfig};
use crate::{Diagnostic, Diagnostics, Hash};

#[derive(Debug, Clone)]
pub struct Program {
    pub script_name: String,
    pub locale: String,
    pub config: Config,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone)]
pub enum Item {
    Narration { id: String, text: String, source_hash: Hash, config: Config },
    Action {
        block_id: String,
        scene: String,
        body: String,
        config: Config,
        policy: String,
        align: String,
    },
    Pause { ms: u64 },
}

pub fn resolve(
    script: &Script,
    script_name: &str,
    locale: &str,
    project: &PartialConfig,
    cli: &PartialConfig,
) -> Result<Program, Diagnostics> {
    let mut diags = Vec::new();

    let front = match PartialConfig::from_yaml(&script.front_matter) {
        Ok(f) => f,
        Err(e) => {
            diags.push(Diagnostic::error(format!("front matter: {e}")));
            PartialConfig::default()
        }
    };

    let base = Config::merged(&[project.clone(), front.clone(), cli.clone()]);
    let mut items = Vec::new();

    for chapter in &script.chapters {
        for node in &chapter.nodes {
            match node {
                Node::Segment(seg) => {
                    let (attrs, mut d) = parse_attrs(&seg.raw_attrs, SEGMENT_KEYS, seg.span);
                    diags.append(&mut d);
                    let config = Config::merged(&[
                        project.clone(),
                        front.clone(),
                        PartialConfig::from_attrs(&attrs),
                        cli.clone(),
                    ]);
                    let text = seg.text.clone();
                    items.push(Item::Narration {
                        id: seg.id.clone().unwrap_or_default(),
                        source_hash: Hash::of(text.as_bytes()),
                        text,
                        config,
                    });
                }
                Node::ActionBlock(block) => {
                    let (attrs, mut d) = parse_attrs(&block.info, BLOCK_KEYS, block.span);
                    diags.append(&mut d);
                    let Some(scene) = attrs.get("scene").map(str::to_string) else {
                        diags.push(
                            Diagnostic::error("action block has no `scene`")
                                .at(block.span)
                                .with_help("write ```teleprompt scene=browser"),
                        );
                        continue;
                    };
                    let config = Config::merged(&[
                        project.clone(),
                        front.clone(),
                        PartialConfig::from_attrs(&attrs),
                        cli.clone(),
                    ]);
                    items.push(Item::Action {
                        block_id: block.id.clone().unwrap_or_default(),
                        scene,
                        body: block.body.clone(),
                        policy: attrs.get("policy").unwrap_or("hold").to_string(),
                        align: attrs.get("align").unwrap_or("start").to_string(),
                        config,
                    });
                }
                Node::Directive(Directive::Pause(ms)) => items.push(Item::Pause { ms: *ms }),
            }
        }
    }

    let d = Diagnostics(diags);
    if d.has_errors() {
        return Err(d);
    }

    Ok(Program {
        script_name: script_name.to_string(),
        locale: locale.to_string(),
        config: base,
        items,
    })
}
```

`PartialConfig` needs `#[derive(Clone)]`, which it already has from Task 5.

```rust
// crates/teleprompt-core/src/lib.rs — add
pub mod program;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p teleprompt-core`
Expected: PASS, 49 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/teleprompt-core
git commit -m "feat(core): resolve scripts into locale-specific programs"
```

---

### Task 9: Beats and policy layout

**Files:**
- Create: `crates/teleprompt-schedule/Cargo.toml`, `src/lib.rs`, `src/beat.rs`, `src/policy.rs`
- Test: `crates/teleprompt-schedule/tests/policy.rs`

**Interfaces:**
- Consumes: `Hash` (Task 1), `TimingConfig` (Task 5), `VoiceSource` (Task 7).
- Produces:

```rust
pub enum Align { Start, End, Center }
pub enum Policy { Hold, Concurrent(Align), Stretch, Trim }
impl Policy { pub fn parse(policy: &str, align: &str) -> Option<Policy>; pub fn label(&self) -> &'static str; }

pub struct Layout {
    pub narration_start_ms: u64,
    pub action_start_ms: u64,
    pub action_duration_ms: u64,   // after stretch/trim adjustment
    pub beat_duration_ms: u64,
    pub warnings: Vec<String>,
}

pub fn layout(policy: Policy, narration_ms: u64, action_ms: u64, timing: &TimingConfig) -> Layout;
```

Semantics, exactly as specified in §6.2. `narration_ms` is already padded with `lead_in` and `tail` by the caller.

| policy | narration | action | beat |
|---|---|---|---|
| `Hold` | `[0, n]` | `[n, n+a]` | `n + a` |
| `Concurrent(Start)` | `[0, n]` | `[0, a]` | `max(n, a)` |
| `Concurrent(End)` | `[beat-n, beat]` | `[beat-a, beat]` | `max(n, a)` |
| `Concurrent(Center)` | centred | centred | `max(n, a)` |
| `Stretch` | `[0, n]` | `[0, a']` where `a' = a * clamp(n/a, min, max)` | `max(n, a')` |
| `Trim` | `[0, n]` | `[0, a']` where `a' = max(n, a / clamp(a/n, 1, max_speedup))` capped at `n` | `n` |

- [ ] **Step 1: Write the failing tests**

```rust
// crates/teleprompt-schedule/tests/policy.rs
use teleprompt_core::config::TimingConfig;
use teleprompt_schedule::{layout, Align, Policy};

fn timing() -> TimingConfig {
    TimingConfig {
        lead_in_ms: 150,
        tail_ms: 150,
        max_stretch: 3.0,
        min_stretch: 0.33,
        max_speedup: 2.0,
    }
}

#[test]
fn hold_runs_the_action_after_the_narration() {
    let l = layout(Policy::Hold, 5000, 800, &timing());
    assert_eq!(l.narration_start_ms, 0);
    assert_eq!(l.action_start_ms, 5000);
    assert_eq!(l.action_duration_ms, 800);
    assert_eq!(l.beat_duration_ms, 5800);
}

#[test]
fn hold_with_no_action_is_just_the_narration() {
    let l = layout(Policy::Hold, 5000, 0, &timing());
    assert_eq!(l.beat_duration_ms, 5000);
}

#[test]
fn concurrent_start_begins_both_together() {
    let l = layout(Policy::Concurrent(Align::Start), 5000, 800, &timing());
    assert_eq!(l.narration_start_ms, 0);
    assert_eq!(l.action_start_ms, 0);
    assert_eq!(l.beat_duration_ms, 5000, "the longer of the two");
}

#[test]
fn concurrent_start_uses_the_action_when_it_is_longer() {
    let l = layout(Policy::Concurrent(Align::Start), 800, 5000, &timing());
    assert_eq!(l.beat_duration_ms, 5000);
}

#[test]
fn concurrent_end_finishes_both_together() {
    let l = layout(Policy::Concurrent(Align::End), 5000, 800, &timing());
    assert_eq!(l.narration_start_ms, 0);
    assert_eq!(l.action_start_ms, 4200);
    assert_eq!(l.action_start_ms + l.action_duration_ms, l.beat_duration_ms);
}

#[test]
fn concurrent_center_centres_the_shorter_one() {
    let l = layout(Policy::Concurrent(Align::Center), 5000, 800, &timing());
    assert_eq!(l.action_start_ms, 2100, "(5000 - 800) / 2");
    assert_eq!(l.narration_start_ms, 0);
}

#[test]
fn stretch_makes_the_action_exactly_fill_the_narration() {
    let l = layout(Policy::Stretch, 5000, 2500, &timing());
    assert_eq!(l.action_duration_ms, 5000);
    assert_eq!(l.beat_duration_ms, 5000);
    assert!(l.warnings.is_empty());
}

#[test]
fn stretch_compresses_a_long_action_too() {
    let l = layout(Policy::Stretch, 2000, 4000, &timing());
    assert_eq!(l.action_duration_ms, 2000);
}

#[test]
fn stretch_beyond_the_maximum_is_clamped_and_warns() {
    // 500ms action asked to fill 5000ms is a factor of 10, above max_stretch 3.0
    let l = layout(Policy::Stretch, 5000, 500, &timing());
    assert_eq!(l.action_duration_ms, 1500, "500 * 3.0");
    assert_eq!(l.beat_duration_ms, 5000, "narration still governs");
    assert!(l.warnings[0].contains("max_stretch"));
}

#[test]
fn stretch_below_the_minimum_is_clamped_and_warns() {
    // 10000ms action asked to fit 1000ms is a factor of 0.1, below min_stretch 0.33
    let l = layout(Policy::Stretch, 1000, 10_000, &timing());
    assert_eq!(l.action_duration_ms, 3300, "10000 * 0.33");
    assert_eq!(l.beat_duration_ms, 3300, "the clamped action now governs");
    assert!(l.warnings[0].contains("min_stretch"));
}

#[test]
fn stretch_with_a_zero_length_action_does_not_divide_by_zero() {
    let l = layout(Policy::Stretch, 5000, 0, &timing());
    assert_eq!(l.action_duration_ms, 0);
    assert_eq!(l.beat_duration_ms, 5000);
}

#[test]
fn trim_leaves_a_short_action_alone() {
    let l = layout(Policy::Trim, 5000, 800, &timing());
    assert_eq!(l.action_duration_ms, 800);
    assert_eq!(l.beat_duration_ms, 5000, "narration is authoritative");
}

#[test]
fn trim_speeds_up_an_over_long_action_within_the_bound() {
    let l = layout(Policy::Trim, 5000, 8000, &timing());
    assert_eq!(l.action_duration_ms, 5000, "8000 / 1.6 fits exactly");
    assert!(l.warnings.is_empty());
}

#[test]
fn trim_beyond_max_speedup_cuts_and_warns() {
    // 20000ms into 5000ms needs 4x, above max_speedup 2.0
    let l = layout(Policy::Trim, 5000, 20_000, &timing());
    assert_eq!(l.action_duration_ms, 5000, "cut to the narration length");
    assert_eq!(l.beat_duration_ms, 5000);
    assert!(l.warnings[0].contains("max_speedup"));
}

#[test]
fn trim_with_zero_narration_does_not_divide_by_zero() {
    let l = layout(Policy::Trim, 0, 5000, &timing());
    assert_eq!(l.beat_duration_ms, 0);
    assert_eq!(l.action_duration_ms, 0);
}

#[test]
fn policy_parses_from_attribute_strings() {
    assert_eq!(Policy::parse("hold", "start"), Some(Policy::Hold));
    assert_eq!(Policy::parse("concurrent", "end"), Some(Policy::Concurrent(Align::End)));
    assert_eq!(Policy::parse("stretch", "start"), Some(Policy::Stretch));
    assert_eq!(Policy::parse("trim", "start"), Some(Policy::Trim));
    assert_eq!(Policy::parse("nonsense", "start"), None);
    assert_eq!(Policy::parse("concurrent", "sideways"), None);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p teleprompt-schedule`
Expected: FAIL — crate does not exist.

- [ ] **Step 3: Create the crate**

```toml
# crates/teleprompt-schedule/Cargo.toml
[package]
name = "teleprompt-schedule"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
teleprompt-core = { path = "../teleprompt-core" }
teleprompt-voice = { path = "../teleprompt-voice" }
serde.workspace = true
serde_json.workspace = true

[dev-dependencies]
insta.workspace = true
```

- [ ] **Step 4: Implement policy layout**

```rust
// crates/teleprompt-schedule/src/policy.rs
use teleprompt_core::config::TimingConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Start,
    End,
    Center,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    Hold,
    Concurrent(Align),
    Stretch,
    Trim,
}

impl Policy {
    pub fn parse(policy: &str, align: &str) -> Option<Self> {
        let align = match align {
            "start" => Align::Start,
            "end" => Align::End,
            "center" => Align::Center,
            _ => return None,
        };
        match policy {
            "hold" => Some(Policy::Hold),
            "concurrent" => Some(Policy::Concurrent(align)),
            "stretch" => Some(Policy::Stretch),
            "trim" => Some(Policy::Trim),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Policy::Hold => "hold",
            Policy::Concurrent(_) => "concurrent",
            Policy::Stretch => "stretch",
            Policy::Trim => "trim",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub narration_start_ms: u64,
    pub action_start_ms: u64,
    pub action_duration_ms: u64,
    pub beat_duration_ms: u64,
    pub warnings: Vec<String>,
}

pub fn layout(policy: Policy, narration_ms: u64, action_ms: u64, timing: &TimingConfig) -> Layout {
    match policy {
        Policy::Hold => Layout {
            narration_start_ms: 0,
            action_start_ms: narration_ms,
            action_duration_ms: action_ms,
            beat_duration_ms: narration_ms + action_ms,
            warnings: Vec::new(),
        },

        Policy::Concurrent(align) => {
            let beat = narration_ms.max(action_ms);
            let (n_start, a_start) = match align {
                Align::Start => (0, 0),
                Align::End => (beat - narration_ms, beat - action_ms),
                Align::Center => ((beat - narration_ms) / 2, (beat - action_ms) / 2),
            };
            Layout {
                narration_start_ms: n_start,
                action_start_ms: a_start,
                action_duration_ms: action_ms,
                beat_duration_ms: beat,
                warnings: Vec::new(),
            }
        }

        Policy::Stretch => {
            let mut warnings = Vec::new();
            let adjusted = if action_ms == 0 {
                0
            } else {
                let wanted = narration_ms as f64 / action_ms as f64;
                let factor = if wanted > timing.max_stretch {
                    warnings.push(format!(
                        "stretch factor {wanted:.2} exceeds max_stretch {:.2}; clamped",
                        timing.max_stretch
                    ));
                    timing.max_stretch
                } else if wanted < timing.min_stretch {
                    warnings.push(format!(
                        "stretch factor {wanted:.2} is below min_stretch {:.2}; clamped",
                        timing.min_stretch
                    ));
                    timing.min_stretch
                } else {
                    wanted
                };
                (action_ms as f64 * factor).round() as u64
            };
            Layout {
                narration_start_ms: 0,
                action_start_ms: 0,
                action_duration_ms: adjusted,
                beat_duration_ms: narration_ms.max(adjusted),
                warnings,
            }
        }

        Policy::Trim => {
            let mut warnings = Vec::new();
            let adjusted = if narration_ms == 0 {
                0
            } else if action_ms <= narration_ms {
                action_ms
            } else {
                let needed = action_ms as f64 / narration_ms as f64;
                if needed > timing.max_speedup {
                    warnings.push(format!(
                        "action needs {needed:.2}x speedup, above max_speedup {:.2}; cut to fit",
                        timing.max_speedup
                    ));
                }
                narration_ms
            };
            Layout {
                narration_start_ms: 0,
                action_start_ms: 0,
                action_duration_ms: adjusted,
                beat_duration_ms: narration_ms,
                warnings,
            }
        }
    }
}
```

```rust
// crates/teleprompt-schedule/src/lib.rs
pub mod beat;
pub mod policy;

pub use beat::{ActionInput, Beat, NarrationInput};
pub use policy::{layout, Align, Layout, Policy};
```

- [ ] **Step 5: Implement the beat types**

```rust
// crates/teleprompt-schedule/src/beat.rs
use teleprompt_core::config::Config;
use teleprompt_core::Hash;
use teleprompt_voice::VoiceSource;

use crate::policy::Policy;

#[derive(Debug, Clone)]
pub struct NarrationInput {
    pub segment_id: String,
    pub source_hash: Hash,
    pub audio_hash: Hash,
    /// Clip duration excluding lead-in and tail padding.
    pub duration_ms: u64,
    pub voice_source: VoiceSource,
    pub voice_source_actual: VoiceSource,
    pub downgrade_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationSource {
    Exact,
    Estimated,
    Measured,
}

#[derive(Debug, Clone)]
pub struct ActionInput {
    pub span_id: String,
    pub scene: String,
    pub adapter: String,
    pub span_hash: Hash,
    pub duration_ms: u64,
    pub duration_source: DurationSource,
}

#[derive(Debug, Clone)]
pub struct Beat {
    pub id: String,
    pub narration: Option<NarrationInput>,
    pub action: Option<ActionInput>,
    pub policy: Policy,
    pub config: Config,
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p teleprompt-schedule`
Expected: PASS, 16 tests.

- [ ] **Step 7: Commit**

```bash
git add crates/teleprompt-schedule
git commit -m "feat(schedule): beat types and the four policy layouts"
```

---

### Task 10: The Timeline and the schedule function

**Files:**
- Create: `crates/teleprompt-schedule/src/timeline.rs`, `crates/teleprompt-schedule/src/schedule.rs`
- Modify: `crates/teleprompt-schedule/src/lib.rs`
- Test: `crates/teleprompt-schedule/tests/schedule.rs`

**Interfaces:**
- Consumes: `Beat`, `layout`, `Policy` (Task 9); `TransitionConfig`, `TransitionDuration` (Task 5).
- Produces:

```rust
pub struct Timeline { pub version: u32, pub script: String, pub locale: String, pub duration_ms: u64, pub generated_by: String, pub entries: Vec<Entry> }
pub struct Entry { pub beat: String, pub start_ms: u64, pub duration_ms: u64, pub policy: String, pub narration: Option<NarrationEntry>, pub action: Option<ActionEntry>, pub transition: TransitionEntry }
pub fn schedule(beats: &[Beat], script: &str, locale: &str, version: &str) -> (Timeline, Vec<String>);
```

Transitions overlap: `entries[i+1].start_ms == entries[i].start_ms + entries[i].duration_ms - transition.duration_ms`. Total duration accounts for the overlaps. `auto` duration is `clamp(slack / 2, min_ms, max_ms)` where `slack = narration_padded - action_adjusted`, floored at 0.

- [ ] **Step 1: Write the failing tests**

```rust
// crates/teleprompt-schedule/tests/schedule.rs
use teleprompt_core::config::{Config, TransitionDuration};
use teleprompt_core::Hash;
use teleprompt_schedule::{schedule, ActionInput, Beat, DurationSource, NarrationInput, Policy};
use teleprompt_voice::VoiceSource;

fn narration(id: &str, ms: u64) -> NarrationInput {
    NarrationInput {
        segment_id: id.into(),
        source_hash: Hash::of(id.as_bytes()),
        audio_hash: Hash::of(id.as_bytes()),
        duration_ms: ms,
        voice_source: VoiceSource::Synthetic,
        voice_source_actual: VoiceSource::Synthetic,
        downgrade_reason: None,
    }
}

fn action(id: &str, ms: u64) -> ActionInput {
    ActionInput {
        span_id: id.into(),
        scene: "mock".into(),
        adapter: "mock".into(),
        span_hash: Hash::of(id.as_bytes()),
        duration_ms: ms,
        duration_source: DurationSource::Exact,
    }
}

fn beat(id: &str, n: Option<u64>, a: Option<u64>, policy: Policy, cfg: Config) -> Beat {
    Beat {
        id: id.into(),
        narration: n.map(|ms| narration(id, ms)),
        action: a.map(|ms| action(id, ms)),
        policy,
        config: cfg,
    }
}

fn no_transition() -> Config {
    let mut c = Config::default();
    c.transition.duration = TransitionDuration::Fixed(0);
    c
}

#[test]
fn narration_is_padded_with_lead_in_and_tail() {
    let t = schedule(&[beat("b1", Some(1000), None, Policy::Hold, no_transition())], "s.md", "en", "0.1.0").0;
    // 150 lead-in + 1000 clip + 150 tail
    assert_eq!(t.entries[0].duration_ms, 1300);
    assert_eq!(t.entries[0].narration.as_ref().unwrap().duration_ms, 1000);
}

#[test]
fn beats_lay_out_sequentially_when_transitions_are_zero() {
    let cfg = no_transition();
    let beats = vec![
        beat("b1", Some(1000), None, Policy::Hold, cfg.clone()),
        beat("b2", Some(2000), None, Policy::Hold, cfg),
    ];
    let t = schedule(&beats, "s.md", "en", "0.1.0").0;
    assert_eq!(t.entries[0].start_ms, 0);
    assert_eq!(t.entries[1].start_ms, 1300);
    assert_eq!(t.duration_ms, 1300 + 2300);
}

#[test]
fn transitions_overlap_adjacent_beats() {
    let mut cfg = Config::default();
    cfg.transition.duration = TransitionDuration::Fixed(300);
    let beats = vec![
        beat("b1", Some(1000), None, Policy::Hold, cfg.clone()),
        beat("b2", Some(1000), None, Policy::Hold, cfg),
    ];
    let t = schedule(&beats, "s.md", "en", "0.1.0").0;
    assert_eq!(t.entries[0].duration_ms, 1300);
    assert_eq!(t.entries[1].start_ms, 1000, "1300 - 300 overlap");
    assert_eq!(t.duration_ms, 2300, "1300 + 1300 - 300");
}

#[test]
fn the_last_beat_has_no_outgoing_transition() {
    let mut cfg = Config::default();
    cfg.transition.duration = TransitionDuration::Fixed(300);
    let t = schedule(&[beat("b1", Some(1000), None, Policy::Hold, cfg)], "s.md", "en", "0.1.0").0;
    assert_eq!(t.entries[0].transition.duration_ms, 0);
}

#[test]
fn auto_transition_is_half_the_slack_clamped_to_the_maximum() {
    let cfg = Config::default(); // auto, min 0, max 600
    let beats = vec![
        beat("b1", Some(5000), Some(200), Policy::Concurrent(teleprompt_schedule::Align::Start), cfg.clone()),
        beat("b2", Some(1000), None, Policy::Hold, cfg),
    ];
    let t = schedule(&beats, "s.md", "en", "0.1.0").0;
    // slack = 5300 padded narration - 200 action = 5100; half is 2550, clamped to 600
    assert_eq!(t.entries[0].transition.duration_ms, 600);
}

#[test]
fn auto_transition_is_zero_when_there_is_no_slack() {
    let cfg = Config::default();
    let beats = vec![
        beat("b1", Some(1000), Some(5000), Policy::Concurrent(teleprompt_schedule::Align::Start), cfg.clone()),
        beat("b2", Some(1000), None, Policy::Hold, cfg),
    ];
    let t = schedule(&beats, "s.md", "en", "0.1.0").0;
    assert_eq!(t.entries[0].transition.duration_ms, 0, "action outlasts narration");
}

#[test]
fn action_offsets_are_absolute_not_beat_relative() {
    let cfg = no_transition();
    let beats = vec![
        beat("b1", Some(1000), None, Policy::Hold, cfg.clone()),
        beat("b2", Some(1000), Some(500), Policy::Hold, cfg),
    ];
    let t = schedule(&beats, "s.md", "en", "0.1.0").0;
    let a = t.entries[1].action.as_ref().unwrap();
    assert_eq!(a.start_ms, 1300 + 1300, "beat 2 start + narration");
}

#[test]
fn policy_warnings_are_collected_with_the_beat_id() {
    let mut cfg = no_transition();
    cfg.timing.max_stretch = 2.0;
    let t = schedule(&[beat("b1", Some(10_000), Some(500), Policy::Stretch, cfg)], "s.md", "en", "0.1.0");
    assert!(t.1[0].contains("b1"));
    assert!(t.1[0].contains("max_stretch"));
}

#[test]
fn an_action_only_beat_schedules_with_no_narration() {
    let t = schedule(&[beat("b1", None, Some(800), Policy::Hold, no_transition())], "s.md", "en", "0.1.0").0;
    assert!(t.entries[0].narration.is_none());
    assert_eq!(t.entries[0].duration_ms, 800);
}

#[test]
fn an_empty_program_produces_an_empty_timeline() {
    let t = schedule(&[], "s.md", "en", "0.1.0").0;
    assert_eq!(t.duration_ms, 0);
    assert!(t.entries.is_empty());
}

#[test]
fn the_timeline_json_shape_is_stable() {
    let cfg = no_transition();
    let beats = vec![beat("welcome", Some(1000), Some(500), Policy::Hold, cfg)];
    let t = schedule(&beats, "script.md", "nl", "0.1.0").0;
    insta::assert_json_snapshot!(t);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p teleprompt-schedule --test schedule`
Expected: FAIL — `unresolved import teleprompt_schedule::schedule`.

- [ ] **Step 3: Implement the Timeline types**

```rust
// crates/teleprompt-schedule/src/timeline.rs
use serde::Serialize;
use teleprompt_core::Hash;

#[derive(Debug, Clone, Serialize)]
pub struct Timeline {
    pub version: u32,
    pub script: String,
    pub locale: String,
    pub duration_ms: u64,
    pub generated_by: String,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub beat: String,
    pub start_ms: u64,
    pub duration_ms: u64,
    pub policy: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub narration: Option<NarrationEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<ActionEntry>,
    pub transition: TransitionEntry,
}

#[derive(Debug, Clone, Serialize)]
pub struct NarrationEntry {
    pub segment: String,
    pub source_hash: Hash,
    pub audio_hash: Hash,
    pub start_ms: u64,
    pub duration_ms: u64,
    pub voice_source: String,
    pub voice_source_actual: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downgrade_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActionEntry {
    pub span: String,
    pub scene: String,
    pub adapter: String,
    pub span_hash: Hash,
    pub start_ms: u64,
    pub duration_ms: u64,
    pub duration_source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TransitionEntry {
    pub kind: String,
    pub duration_ms: u64,
}

impl Timeline {
    pub fn entry(&self, beat: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.beat == beat)
    }
}
```

- [ ] **Step 4: Implement the schedule function**

```rust
// crates/teleprompt-schedule/src/schedule.rs
use teleprompt_core::config::TransitionDuration;

use crate::beat::{Beat, DurationSource};
use crate::policy::layout;
use crate::timeline::{ActionEntry, Entry, NarrationEntry, Timeline, TransitionEntry};

pub const TIMELINE_VERSION: u32 = 1;

/// Pure: same inputs always produce the same timeline.
/// Returns the timeline plus any policy warnings, tagged with their beat.
pub fn schedule(beats: &[Beat], script: &str, locale: &str, version: &str) -> (Timeline, Vec<String>) {
    let mut entries: Vec<Entry> = Vec::with_capacity(beats.len());
    let mut warnings = Vec::new();
    let mut cursor = 0u64;

    for (i, beat) in beats.iter().enumerate() {
        let timing = &beat.config.timing;

        let narration_ms = beat
            .narration
            .as_ref()
            .map(|n| timing.lead_in_ms + n.duration_ms + timing.tail_ms)
            .unwrap_or(0);
        let action_ms = beat.action.as_ref().map(|a| a.duration_ms).unwrap_or(0);

        let l = layout(beat.policy, narration_ms, action_ms, timing);
        for w in &l.warnings {
            warnings.push(format!("{}: {w}", beat.id));
        }

        let slack = narration_ms.saturating_sub(l.action_duration_ms);
        let is_last = i + 1 == beats.len();
        let transition_ms = if is_last {
            0
        } else {
            match beat.config.transition.duration {
                TransitionDuration::Fixed(ms) => ms,
                TransitionDuration::Auto => (slack / 2)
                    .clamp(beat.config.transition.min_ms, beat.config.transition.max_ms),
            }
        }
        // A transition can never consume more than the beat it leaves.
        .min(l.beat_duration_ms);

        entries.push(Entry {
            beat: beat.id.clone(),
            start_ms: cursor,
            duration_ms: l.beat_duration_ms,
            policy: beat.policy.label().to_string(),
            narration: beat.narration.as_ref().map(|n| NarrationEntry {
                segment: n.segment_id.clone(),
                source_hash: n.source_hash,
                audio_hash: n.audio_hash,
                start_ms: cursor + l.narration_start_ms + timing.lead_in_ms,
                duration_ms: n.duration_ms,
                voice_source: n.voice_source.label().to_string(),
                voice_source_actual: n.voice_source_actual.label().to_string(),
                downgrade_reason: n.downgrade_reason.clone(),
            }),
            action: beat.action.as_ref().map(|a| ActionEntry {
                span: a.span_id.clone(),
                scene: a.scene.clone(),
                adapter: a.adapter.clone(),
                span_hash: a.span_hash,
                start_ms: cursor + l.action_start_ms,
                duration_ms: l.action_duration_ms,
                duration_source: match a.duration_source {
                    DurationSource::Exact => "exact",
                    DurationSource::Estimated => "estimated",
                    DurationSource::Measured => "measured",
                }
                .to_string(),
            }),
            transition: TransitionEntry {
                kind: beat.config.transition.kind.clone(),
                duration_ms: transition_ms,
            },
        });

        cursor += l.beat_duration_ms - transition_ms;
    }

    let duration_ms = entries
        .last()
        .map(|e| e.start_ms + e.duration_ms)
        .unwrap_or(0);

    (
        Timeline {
            version: TIMELINE_VERSION,
            script: script.to_string(),
            locale: locale.to_string(),
            duration_ms,
            generated_by: format!("teleprompt {version}"),
            entries,
        },
        warnings,
    )
}
```

```rust
// crates/teleprompt-schedule/src/lib.rs
pub mod beat;
pub mod policy;
pub mod schedule;
pub mod timeline;

pub use beat::{ActionInput, Beat, DurationSource, NarrationInput};
pub use policy::{layout, Align, Layout, Policy};
pub use schedule::{schedule, TIMELINE_VERSION};
pub use timeline::{ActionEntry, Entry, NarrationEntry, Timeline, TransitionEntry};
```

- [ ] **Step 5: Run tests and accept the snapshot**

Run: `cargo test -p teleprompt-schedule`
Expected: FAIL once on the snapshot test with a pending `.snap.new` file.

Run: `cargo insta accept` (or `INSTA_UPDATE=always cargo test -p teleprompt-schedule`)
Then run: `cargo test -p teleprompt-schedule`
Expected: PASS, 27 tests.

Review the accepted snapshot by eye before committing: `duration_ms` should be 1650 (150 + 1000 + 150 narration, then 500 action), and `entries[0].action.start_ms` should be 1300.

- [ ] **Step 6: Commit**

```bash
git add crates/teleprompt-schedule
git commit -m "feat(schedule): timeline computation with overlapping transitions"
```

---

### Task 11: Compiling a program into beats

**Files:**
- Create: `crates/teleprompt-compile/Cargo.toml`, `crates/teleprompt-compile/src/lib.rs`
- Modify: root `Cargo.toml` is unchanged (`members = ["crates/*"]` picks it up)
- Test: `crates/teleprompt-compile/tests/compile.rs`

This crate is the seam where the pure pieces meet: it walks a `Program`, asks the scene registry to validate and split action blocks, asks the voice backend for durations, pairs narration with the following action span into beats, and hands the result to the scheduler. It exists as its own crate so `core`, `scene`, `voice`, and `schedule` never depend on one another.

**Interfaces:**
- Consumes: `Program`, `Item` (Task 8); `SceneRegistry`, `Measured` (Task 6); `VoiceBackend`, `resolve_source`, `VoiceSource` (Task 7); `Beat`, `Policy`, `schedule` (Tasks 9–10).
- Produces:

```rust
pub struct CompileOutput { pub timeline: Timeline, pub warnings: Vec<String> }
pub fn compile(
    program: &Program,
    registry: &SceneRegistry,
    voice: &dyn VoiceBackend,
    version: &str,
) -> Result<CompileOutput, Diagnostics>;
```

Beat pairing rule: a `Narration` item opens a beat; the immediately following `Action` item's **first span** joins it, and each remaining span of that block becomes its own action-only beat. A `Pause` item becomes a narration-less, action-less beat of the stated duration. An `Action` with no preceding narration is an action-only beat.

- [ ] **Step 1: Write the failing tests**

```rust
// crates/teleprompt-compile/tests/compile.rs
use teleprompt_compile::compile;
use teleprompt_core::config::PartialConfig;
use teleprompt_core::ident::assign_ids;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::{resolve, Program};
use teleprompt_scene::SceneRegistry;
use teleprompt_voice::NullVoice;

fn program(src: &str) -> Program {
    let mut s = parse_script(src).unwrap();
    assign_ids(&mut s);
    resolve(&s, "demo.md", "en", &PartialConfig::default(), &PartialConfig::default()).unwrap()
}

fn run(src: &str) -> teleprompt_compile::CompileOutput {
    compile(&program(src), &SceneRegistry::with_builtins(), &NullVoice::default(), "0.1.0").unwrap()
}

const ONE_BEAT: &str = r#"---
scene: { mock: { adapter: mock } }
transition: { duration: 0ms }
---

# Intro

One two three four five six. {#welcome}

```teleprompt scene=mock
wait 500ms
```
"#;

#[test]
fn narration_and_the_following_action_form_one_beat() {
    let out = run(ONE_BEAT);
    assert_eq!(out.timeline.entries.len(), 1);
    let e = &out.timeline.entries[0];
    assert!(e.narration.is_some());
    assert!(e.action.is_some());
}

#[test]
fn narration_duration_comes_from_the_voice_backend() {
    let out = run(ONE_BEAT);
    let n = out.timeline.entries[0].narration.as_ref().unwrap();
    // 6 words at 150 wpm = 2400ms, plus 350ms for the full stop
    assert_eq!(n.duration_ms, 2750);
}

#[test]
fn action_duration_comes_from_the_scene_estimate() {
    let out = run(ONE_BEAT);
    assert_eq!(out.timeline.entries[0].action.as_ref().unwrap().duration_ms, 500);
    assert_eq!(out.timeline.entries[0].action.as_ref().unwrap().duration_source, "exact");
}

#[test]
fn each_mark_after_the_first_span_becomes_its_own_beat() {
    let src = r#"---
scene: { mock: { adapter: mock } }
---

# Intro

One. {#a}

```teleprompt scene=mock
wait 100ms
mark
wait 200ms
mark
wait 300ms
```
"#;
    let out = run(src);
    assert_eq!(out.timeline.entries.len(), 3);
    assert!(out.timeline.entries[0].narration.is_some());
    assert!(out.timeline.entries[1].narration.is_none());
    assert_eq!(out.timeline.entries[1].action.as_ref().unwrap().duration_ms, 200);
    assert_eq!(out.timeline.entries[2].action.as_ref().unwrap().duration_ms, 300);
}

#[test]
fn a_pause_directive_becomes_a_silent_beat() {
    let src = "---\nscene: { mock: { adapter: mock } }\n---\n\n# A\n\nOne. {#a}\n\n<!-- teleprompt: pause 800ms -->\n";
    let out = run(src);
    assert_eq!(out.timeline.entries.len(), 2);
    let p = &out.timeline.entries[1];
    assert_eq!(p.duration_ms, 800);
    assert!(p.narration.is_none(), "a pause is silent");
    assert_eq!(p.action.as_ref().unwrap().scene, "pause");
}

#[test]
fn the_scene_records_both_its_name_and_its_adapter() {
    let out = run(ONE_BEAT);
    let a = out.timeline.entries[0].action.as_ref().unwrap();
    assert_eq!(a.scene, "mock");
    assert_eq!(a.adapter, "mock");
}

#[test]
fn an_unconfigured_scene_falls_back_to_its_default_adapter() {
    let src = "# A\n\nOne. {#a}\n\n```teleprompt scene=mock\nwait 100ms\n```\n";
    let out = run(src);
    assert_eq!(out.timeline.entries[0].action.as_ref().unwrap().adapter, "mock");
}

#[test]
fn an_unavailable_adapter_is_a_diagnostic_not_a_panic() {
    let src = "# A\n\nOne. {#a}\n\n```teleprompt scene=browser\nawait page.goto('/');\n```\n";
    let p = program(src);
    let e = compile(&p, &SceneRegistry::with_builtins(), &NullVoice::default(), "0.1.0")
        .unwrap_err();
    assert!(e.0[0].message.contains("no adapter `playwright`"));
    assert!(e.0[0].help.as_deref().unwrap().contains("mock"));
}

#[test]
fn adapter_validation_errors_reach_the_caller() {
    let src = "# A\n\nOne. {#a}\n\n```teleprompt scene=mock\nclick everything\n```\n";
    let p = program(src);
    let e = compile(&p, &SceneRegistry::with_builtins(), &NullVoice::default(), "0.1.0")
        .unwrap_err();
    assert!(e.0[0].message.contains("unknown mock directive"));
}

#[test]
fn an_invalid_policy_is_a_diagnostic() {
    let src = "# A\n\nOne. {#a}\n\n```teleprompt scene=mock policy=sideways\nwait 1s\n```\n";
    let p = program(src);
    let e = compile(&p, &SceneRegistry::with_builtins(), &NullVoice::default(), "0.1.0")
        .unwrap_err();
    assert!(e.0[0].message.contains("unknown policy `sideways`"));
}

#[test]
fn the_null_backend_downgrades_a_recorded_request_and_says_why() {
    let src = "---\nscene: { mock: { adapter: mock } }\n---\n\n# A\n\nOne. {#a voice.source=recorded}\n";
    let out = run(src);
    let n = out.timeline.entries[0].narration.as_ref().unwrap();
    assert_eq!(n.voice_source, "recorded");
    assert_eq!(n.voice_source_actual, "synthetic");
    assert!(n.downgrade_reason.as_ref().unwrap().contains("no takes"));
}

#[test]
fn compilation_is_deterministic() {
    let a = run(ONE_BEAT);
    let b = run(ONE_BEAT);
    assert_eq!(
        serde_json::to_string(&a.timeline).unwrap(),
        serde_json::to_string(&b.timeline).unwrap()
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p teleprompt-compile`
Expected: FAIL — crate does not exist.

- [ ] **Step 3: Create the crate**

```toml
# crates/teleprompt-compile/Cargo.toml
[package]
name = "teleprompt-compile"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
teleprompt-core = { path = "../teleprompt-core" }
teleprompt-scene = { path = "../teleprompt-scene" }
teleprompt-schedule = { path = "../teleprompt-schedule" }
teleprompt-voice = { path = "../teleprompt-voice" }

[dev-dependencies]
serde_json.workspace = true
```

- [ ] **Step 4: Implement compilation**

```rust
// crates/teleprompt-compile/src/lib.rs
use teleprompt_core::config::default_adapter;
use teleprompt_core::program::{Item, Program};
use teleprompt_core::{Diagnostic, Diagnostics, Hash, SourceSpan};
use teleprompt_scene::{BlockSource, SceneRegistry};
use teleprompt_schedule::{
    schedule, ActionInput, Beat, DurationSource, NarrationInput, Policy, Timeline,
};
use teleprompt_voice::{resolve_source, SynthRequest, VoiceBackend, VoiceSource};

pub struct CompileOutput {
    pub timeline: Timeline,
    pub warnings: Vec<String>,
}

pub fn compile(
    program: &Program,
    registry: &SceneRegistry,
    voice: &dyn VoiceBackend,
    version: &str,
) -> Result<CompileOutput, Diagnostics> {
    let mut diags = Vec::new();
    let mut beats: Vec<Beat> = Vec::new();
    let mut pending: Option<NarrationInput> = None;
    let mut pending_id = String::new();
    let mut pending_config = program.config.clone();

    let flush = |beats: &mut Vec<Beat>,
                 pending: &mut Option<NarrationInput>,
                 id: &mut String,
                 cfg: &teleprompt_core::config::Config| {
        if let Some(n) = pending.take() {
            beats.push(Beat {
                id: std::mem::take(id),
                narration: Some(n),
                action: None,
                policy: Policy::Hold,
                config: cfg.clone(),
            });
        }
    };

    for item in &program.items {
        match item {
            Item::Narration { id, text, source_hash, config } => {
                flush(&mut beats, &mut pending, &mut pending_id, &pending_config);

                let requested = VoiceSource::parse(&config.voice.source).unwrap_or(VoiceSource::Synthetic);
                let resolution = match resolve_source(requested, &|tier| match tier {
                    VoiceSource::Recorded => Err("no takes recorded (M0 has no recorder)".into()),
                    VoiceSource::Cloned => Err("no voice profile enrolled".into()),
                    VoiceSource::Synthetic => Ok(()),
                }) {
                    Ok(r) => r,
                    Err(e) => {
                        diags.push(Diagnostic::error(e));
                        continue;
                    }
                };

                let req = SynthRequest {
                    text: text.clone(),
                    locale: program.locale.clone(),
                    voice: config.voice.voice.clone(),
                    speed: config.voice.speed,
                };
                let synth = match voice.synthesize(&req) {
                    Ok(s) => s,
                    Err(e) => {
                        diags.push(Diagnostic::error(format!("segment `{id}`: {e}")));
                        continue;
                    }
                };

                pending = Some(NarrationInput {
                    segment_id: id.clone(),
                    source_hash: *source_hash,
                    audio_hash: synth.audio_hash,
                    duration_ms: synth.duration_ms,
                    voice_source: resolution.requested,
                    voice_source_actual: resolution.actual,
                    downgrade_reason: resolution.downgrade_reason,
                });
                pending_id = id.clone();
                pending_config = config.clone();
            }

            Item::Action { block_id, scene, body, config, policy, align } => {
                let Some(parsed_policy) = Policy::parse(policy, align) else {
                    diags.push(
                        Diagnostic::error(format!("unknown policy `{policy}` or align `{align}`"))
                            .with_help("policy is hold|concurrent|stretch|trim; align is start|end|center"),
                    );
                    continue;
                };

                let adapter_name = config
                    .scenes
                    .get(scene)
                    .map(|s| s.adapter.clone())
                    .unwrap_or_else(|| default_adapter(scene).to_string());

                let Some(adapter) = registry.get(&adapter_name) else {
                    diags.push(
                        Diagnostic::error(format!(
                            "scene `{scene}` needs adapter `{adapter_name}`, but no adapter `{adapter_name}` is available"
                        ))
                        .with_help(format!("available adapters: {}", registry.available().join(", "))),
                    );
                    continue;
                };

                let src = BlockSource {
                    scene: scene.clone(),
                    body: body.clone(),
                    span: SourceSpan { line: 0, column: 1, len: 0 },
                };
                let validated = match adapter.validate(&src) {
                    Ok(v) => v,
                    Err(mut e) => {
                        diags.append(&mut e);
                        continue;
                    }
                };
                let spans = match adapter.spans(&validated, block_id) {
                    Ok(s) => s,
                    Err(mut e) => {
                        diags.append(&mut e);
                        continue;
                    }
                };

                for (i, span) in spans.iter().enumerate() {
                    let measured = adapter.estimate(span);
                    let action = ActionInput {
                        span_id: span.id.clone(),
                        scene: scene.clone(),
                        adapter: adapter_name.clone(),
                        span_hash: span.hash,
                        duration_ms: measured.duration_ms().unwrap_or(0),
                        duration_source: match measured {
                            teleprompt_scene::Measured::Exact(_) => DurationSource::Exact,
                            teleprompt_scene::Measured::Estimated(_) => DurationSource::Estimated,
                            teleprompt_scene::Measured::Unknown => DurationSource::Estimated,
                        },
                    };

                    if i == 0 && pending.is_some() {
                        beats.push(Beat {
                            id: std::mem::take(&mut pending_id),
                            narration: pending.take(),
                            action: Some(action),
                            policy: parsed_policy,
                            config: config.clone(),
                        });
                    } else {
                        beats.push(Beat {
                            id: span.id.clone(),
                            narration: None,
                            action: Some(action),
                            policy: parsed_policy,
                            config: config.clone(),
                        });
                    }
                }
            }

            Item::Pause { ms } => {
                flush(&mut beats, &mut pending, &mut pending_id, &pending_config);
                beats.push(Beat {
                    id: format!("pause-{}", beats.len()),
                    narration: None,
                    action: Some(ActionInput {
                        span_id: format!("pause-{}", beats.len()),
                        scene: "pause".into(),
                        adapter: "pause".into(),
                        span_hash: Hash::of(ms.to_string().as_bytes()),
                        duration_ms: *ms,
                        duration_source: DurationSource::Exact,
                    }),
                    policy: Policy::Hold,
                    config: program.config.clone(),
                });
            }
        }
    }

    flush(&mut beats, &mut pending, &mut pending_id, &pending_config);

    let d = Diagnostics(diags);
    if d.has_errors() {
        return Err(d);
    }

    let (timeline, warnings) = schedule(&beats, &program.script_name, &program.locale, version);
    Ok(CompileOutput { timeline, warnings })
}
```

Note: a `Pause` beat carries its duration in the action slot so the scheduler's existing arithmetic applies unchanged, and the timeline marks it `scene: "pause"`. Reusing the action slot keeps the scheduler free of a third case at the cost of slightly less obvious JSON; the pause test asserts exactly this shape.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p teleprompt-compile`
Expected: PASS, 12 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/teleprompt-compile
git commit -m "feat(compile): assemble programs into scheduled timelines"
```

---

### Task 12: Timeline diffing

**Files:**
- Create: `crates/teleprompt-schedule/src/diff.rs`
- Modify: `crates/teleprompt-schedule/src/lib.rs`, `crates/teleprompt-schedule/src/timeline.rs`
- Test: `crates/teleprompt-schedule/tests/diff.rs`

**Interfaces:**
- Consumes: `Timeline`, `Entry` (Task 10).
- Produces:

```rust
pub struct TimelineDiff {
    pub before_ms: u64,
    pub after_ms: u64,
    pub changed: Vec<ChangedBeat>,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub stale_takes: Vec<StaleTake>,
    pub recapture: Vec<String>,
    pub shift_ms: i64,
}
pub struct ChangedBeat { pub beat: String, pub before_ms: u64, pub after_ms: u64, pub reason: String }
pub struct StaleTake { pub segment: String, pub falls_back_to: String }
pub fn diff(before: &Timeline, after: &Timeline) -> TimelineDiff;
impl TimelineDiff { pub fn is_empty(&self) -> bool; pub fn render(&self) -> String; }
```

`Timeline` and its entry types gain `Deserialize` so a committed timeline can be read back.

- [ ] **Step 1: Write the failing tests**

```rust
// crates/teleprompt-schedule/tests/diff.rs
use teleprompt_core::config::{Config, TransitionDuration};
use teleprompt_core::Hash;
use teleprompt_schedule::{diff, schedule, ActionInput, Beat, DurationSource, NarrationInput, Policy};
use teleprompt_voice::VoiceSource;

fn cfg() -> Config {
    let mut c = Config::default();
    c.transition.duration = TransitionDuration::Fixed(0);
    c
}

fn beat(id: &str, narration_ms: u64, source: &str) -> Beat {
    Beat {
        id: id.into(),
        narration: Some(NarrationInput {
            segment_id: id.into(),
            source_hash: Hash::of(source.as_bytes()),
            audio_hash: Hash::of(source.as_bytes()),
            duration_ms: narration_ms,
            voice_source: VoiceSource::Synthetic,
            voice_source_actual: VoiceSource::Synthetic,
            downgrade_reason: None,
        }),
        action: None,
        policy: Policy::Hold,
        config: cfg(),
    }
}

fn stale_beat(id: &str) -> Beat {
    let mut b = beat(id, 1000, id);
    let n = b.narration.as_mut().unwrap();
    n.voice_source = VoiceSource::Recorded;
    n.voice_source_actual = VoiceSource::Cloned;
    n.downgrade_reason = Some("take stale".into());
    b
}

fn timeline(beats: Vec<Beat>) -> teleprompt_schedule::Timeline {
    schedule(&beats, "s.md", "en", "0.1.0").0
}

#[test]
fn identical_timelines_diff_to_nothing() {
    let a = timeline(vec![beat("b1", 1000, "one")]);
    let b = timeline(vec![beat("b1", 1000, "one")]);
    assert!(diff(&a, &b).is_empty());
}

#[test]
fn a_longer_segment_is_reported_with_both_durations() {
    let a = timeline(vec![beat("b1", 4200, "one")]);
    let b = timeline(vec![beat("b1", 5800, "one but longer")]);
    let d = diff(&a, &b);
    assert_eq!(d.changed.len(), 1);
    assert_eq!(d.changed[0].beat, "b1");
    assert_eq!(d.changed[0].before_ms, 4200);
    assert_eq!(d.changed[0].after_ms, 5800);
    assert!(d.changed[0].reason.contains("text edited"));
}

#[test]
fn a_duration_change_without_a_text_change_is_reported_differently() {
    let a = timeline(vec![beat("b1", 4200, "same")]);
    let mut changed = beat("b1", 5000, "same");
    changed.narration.as_mut().unwrap().audio_hash = Hash::of(b"different voice");
    let b = timeline(vec![changed]);
    let d = diff(&a, &b);
    assert!(d.changed[0].reason.contains("audio changed"));
    assert!(!d.changed[0].reason.contains("text edited"));
}

#[test]
fn total_shift_is_the_difference_in_overall_duration() {
    let a = timeline(vec![beat("b1", 4200, "one"), beat("b2", 1000, "two")]);
    let b = timeline(vec![beat("b1", 5800, "one longer"), beat("b2", 1000, "two")]);
    let d = diff(&a, &b);
    assert_eq!(d.shift_ms, 1600);
    assert_eq!(d.after_ms - d.before_ms, 1600);
}

#[test]
fn added_and_removed_beats_are_listed_separately() {
    let a = timeline(vec![beat("b1", 1000, "one")]);
    let b = timeline(vec![beat("b1", 1000, "one"), beat("b2", 1000, "two")]);
    let d = diff(&a, &b);
    assert_eq!(d.added, ["b2"]);
    assert!(d.removed.is_empty());

    let back = diff(&b, &a);
    assert_eq!(back.removed, ["b2"]);
    assert!(back.added.is_empty());
}

#[test]
fn stale_takes_are_reported_with_their_fallback_tier() {
    let a = timeline(vec![beat("b1", 1000, "one")]);
    let b = timeline(vec![stale_beat("b1")]);
    let d = diff(&a, &b);
    assert_eq!(d.stale_takes.len(), 1);
    assert_eq!(d.stale_takes[0].segment, "b1");
    assert_eq!(d.stale_takes[0].falls_back_to, "cloned");
}

#[test]
fn beats_needing_recapture_are_those_whose_action_hash_or_slot_changed() {
    let action = |id: &str, ms: u64| ActionInput {
        span_id: id.into(),
        scene: "mock".into(),
        adapter: "mock".into(),
        span_hash: Hash::of(id.as_bytes()),
        duration_ms: ms,
        duration_source: DurationSource::Exact,
    };
    let mut a1 = beat("b1", 1000, "one");
    a1.action = Some(action("s1", 500));
    let mut b1 = beat("b1", 2000, "one longer");
    b1.action = Some(action("s1", 500));

    let d = diff(&timeline(vec![a1]), &timeline(vec![b1]));
    assert_eq!(d.recapture, ["b1"], "the slot moved even though the span did not");
}

#[test]
fn rendered_output_names_the_script_change_and_the_shift() {
    let a = timeline(vec![beat("welcome", 4200, "one"), beat("next", 1000, "two")]);
    let b = timeline(vec![beat("welcome", 5800, "one longer"), beat("next", 1000, "two")]);
    let rendered = diff(&a, &b).render();
    assert!(rendered.contains("welcome"));
    assert!(rendered.contains("4.2s"));
    assert!(rendered.contains("5.8s"));
    assert!(rendered.contains("+1.6s"));
}

#[test]
fn an_empty_diff_renders_a_single_reassuring_line() {
    let a = timeline(vec![beat("b1", 1000, "one")]);
    assert_eq!(diff(&a, &a).render(), "no timeline changes");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p teleprompt-schedule --test diff`
Expected: FAIL — `unresolved import teleprompt_schedule::diff`.

- [ ] **Step 3: Make the timeline round-trippable**

Add `Deserialize` beside `Serialize` on `Timeline`, `Entry`, `NarrationEntry`, `ActionEntry`, and `TransitionEntry` in `timeline.rs`:

```rust
use serde::{Deserialize, Serialize};
// then on each struct:
#[derive(Debug, Clone, Serialize, Deserialize)]
```

`Hash` needs a `Deserialize` impl to match its `Serialize`:

```rust
// crates/teleprompt-core/src/hash.rs — append
impl<'de> serde::Deserialize<'de> for Hash {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let mut bytes = [0u8; 32];
        if s.len() != 64 {
            return Err(serde::de::Error::custom("hash must be 64 hex characters"));
        }
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
                .map_err(serde::de::Error::custom)?;
        }
        Ok(Hash(bytes))
    }
}
```

- [ ] **Step 4: Implement diffing**

```rust
// crates/teleprompt-schedule/src/diff.rs
use std::collections::BTreeMap;

use crate::timeline::{Entry, Timeline};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ChangedBeat {
    pub beat: String,
    pub before_ms: u64,
    pub after_ms: u64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StaleTake {
    pub segment: String,
    pub falls_back_to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TimelineDiff {
    pub before_ms: u64,
    pub after_ms: u64,
    pub shift_ms: i64,
    pub changed: Vec<ChangedBeat>,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub stale_takes: Vec<StaleTake>,
    pub recapture: Vec<String>,
}

impl TimelineDiff {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty()
            && self.added.is_empty()
            && self.removed.is_empty()
            && self.stale_takes.is_empty()
            && self.recapture.is_empty()
    }

    pub fn render(&self) -> String {
        if self.is_empty() {
            return "no timeline changes".to_string();
        }

        let mut out = format!(
            "{} → {} ({})",
            secs(self.before_ms),
            secs(self.after_ms),
            signed(self.shift_ms)
        );

        if !self.changed.is_empty() {
            out.push_str("\n\n  changed");
            for c in &self.changed {
                out.push_str(&format!(
                    "\n    {:<14} narration {} → {}   ({})",
                    c.beat,
                    secs(c.before_ms),
                    secs(c.after_ms),
                    c.reason
                ));
            }
        }
        if !self.added.is_empty() {
            out.push_str(&format!("\n\n  added\n    {}", self.added.join(", ")));
        }
        if !self.removed.is_empty() {
            out.push_str(&format!("\n\n  removed\n    {}", self.removed.join(", ")));
        }
        if !self.stale_takes.is_empty() {
            out.push_str("\n\n  stale takes");
            for s in &self.stale_takes {
                out.push_str(&format!(
                    "\n    {:<14} build will fall back to: {}",
                    s.segment, s.falls_back_to
                ));
            }
        }
        if !self.recapture.is_empty() {
            out.push_str(&format!(
                "\n\n  re-render\n    {} beat(s) need capture",
                self.recapture.len()
            ));
        }
        out
    }
}

fn secs(ms: u64) -> String {
    format!("{:.1}s", ms as f64 / 1000.0)
}

fn signed(ms: i64) -> String {
    let sign = if ms >= 0 { "+" } else { "-" };
    format!("{sign}{:.1}s", (ms.abs()) as f64 / 1000.0)
}

pub fn diff(before: &Timeline, after: &Timeline) -> TimelineDiff {
    let old: BTreeMap<&str, &Entry> =
        before.entries.iter().map(|e| (e.beat.as_str(), e)).collect();
    let new: BTreeMap<&str, &Entry> =
        after.entries.iter().map(|e| (e.beat.as_str(), e)).collect();

    let mut changed = Vec::new();
    let mut recapture = Vec::new();
    let mut stale_takes = Vec::new();

    for (id, n) in &new {
        if let Some(o) = old.get(id) {
            if let (Some(on), Some(nn)) = (&o.narration, &n.narration) {
                if on.duration_ms != nn.duration_ms {
                    let reason = if on.source_hash != nn.source_hash {
                        "text edited"
                    } else {
                        "audio changed"
                    };
                    changed.push(ChangedBeat {
                        beat: (*id).to_string(),
                        before_ms: on.duration_ms,
                        after_ms: nn.duration_ms,
                        reason: reason.to_string(),
                    });
                }
            }
            if let (Some(oa), Some(na)) = (&o.action, &n.action) {
                if oa.span_hash != na.span_hash || oa.duration_ms != na.duration_ms
                    || o.duration_ms != n.duration_ms
                {
                    recapture.push((*id).to_string());
                }
            }
        }

        if let Some(nn) = &n.narration {
            if nn.voice_source != nn.voice_source_actual {
                stale_takes.push(StaleTake {
                    segment: nn.segment.clone(),
                    falls_back_to: nn.voice_source_actual.clone(),
                });
            }
        }
    }

    let added: Vec<String> = new
        .keys()
        .filter(|k| !old.contains_key(*k))
        .map(|k| (*k).to_string())
        .collect();
    let removed: Vec<String> = old
        .keys()
        .filter(|k| !new.contains_key(*k))
        .map(|k| (*k).to_string())
        .collect();

    TimelineDiff {
        before_ms: before.duration_ms,
        after_ms: after.duration_ms,
        shift_ms: after.duration_ms as i64 - before.duration_ms as i64,
        changed,
        added,
        removed,
        stale_takes,
        recapture,
    }
}
```

```rust
// crates/teleprompt-schedule/src/lib.rs — add
pub mod diff;
pub use diff::{diff, ChangedBeat, StaleTake, TimelineDiff};
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --workspace`
Expected: PASS, 9 new tests in the diff suite; the whole workspace green.

- [ ] **Step 6: Commit**

```bash
git add crates
git commit -m "feat(schedule): timeline diffing with prose rendering"
```

---

### Task 13: CLI skeleton, `new`, and `doctor`

**Files:**
- Create: `crates/teleprompt-cli/Cargo.toml`, `src/main.rs`, `src/output.rs`, `src/cmd/mod.rs`, `src/cmd/new.rs`, `src/cmd/doctor.rs`
- Test: `crates/teleprompt-cli/tests/new.rs`

**Interfaces:**
- Consumes: `SceneRegistry` (Task 6), `NullVoice` (Task 7).
- Produces: the `teleprompt` binary; `Format::{Human, Json}`; `exit_code_for(&Outcome) -> i32`; `scaffold(dir: &Path) -> std::io::Result<Vec<PathBuf>>`; `doctor_report(&SceneRegistry) -> DoctorReport`.

Exit codes are centralised here so no subcommand invents its own.

- [ ] **Step 1: Write the failing tests**

```rust
// crates/teleprompt-cli/tests/new.rs
use teleprompt_cli::cmd::doctor::doctor_report;
use teleprompt_cli::cmd::new::scaffold;
use teleprompt_scene::SceneRegistry;

#[test]
fn scaffold_writes_a_runnable_project() {
    let dir = tempdir();
    let written = scaffold(&dir).unwrap();
    let names: Vec<String> = written
        .iter()
        .map(|p| p.strip_prefix(&dir).unwrap().display().to_string())
        .collect();
    assert!(names.contains(&"teleprompt.toml".to_string()));
    assert!(names.contains(&"scripts/demo.md".to_string()));
    assert!(names.contains(&".gitignore".to_string()));
}

#[test]
fn the_scaffolded_script_compiles() {
    use teleprompt_compile::compile;
    use teleprompt_core::config::PartialConfig;
    use teleprompt_core::ident::assign_ids;
    use teleprompt_core::parse::parse_script;
    use teleprompt_core::program::resolve;
    use teleprompt_voice::NullVoice;

    let dir = tempdir();
    scaffold(&dir).unwrap();
    let src = std::fs::read_to_string(dir.join("scripts/demo.md")).unwrap();

    let mut script = parse_script(&src).expect("scaffolded script must parse");
    assign_ids(&mut script);
    let program = resolve(
        &script,
        "demo.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .expect("scaffolded script must resolve");
    let out = compile(&program, &SceneRegistry::with_builtins(), &NullVoice::default(), "0.1.0")
        .expect("scaffolded script must compile");
    assert!(out.timeline.duration_ms > 0);
}

#[test]
fn scaffold_refuses_to_clobber_an_existing_project() {
    let dir = tempdir();
    scaffold(&dir).unwrap();
    let err = scaffold(&dir).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
}

#[test]
fn the_gitignore_excludes_caches_and_build_output_but_not_timelines() {
    let dir = tempdir();
    scaffold(&dir).unwrap();
    let ignore = std::fs::read_to_string(dir.join(".gitignore")).unwrap();
    assert!(ignore.contains(".teleprompt/cache/"));
    assert!(ignore.contains("build/"));
    assert!(!ignore.contains("timelines/"));
}

#[test]
fn doctor_reports_available_adapters_and_backends() {
    let r = doctor_report(&SceneRegistry::with_builtins());
    assert!(r.adapters.contains(&"mock".to_string()));
    assert!(r.voice_backends.contains(&"null".to_string()));
}

#[test]
fn doctor_notes_that_rendering_is_out_of_scope_rather_than_failing_on_ffmpeg() {
    let r = doctor_report(&SceneRegistry::with_builtins());
    assert!(r.notes.iter().any(|n| n.contains("M0")));
    assert!(r.ok, "a missing ffmpeg must not make doctor fail in M0");
}

fn tempdir() -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!(
        "teleprompt-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p teleprompt-cli`
Expected: FAIL — crate does not exist.

- [ ] **Step 3: Create the crate**

```toml
# crates/teleprompt-cli/Cargo.toml
[package]
name = "teleprompt-cli"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[lib]
name = "teleprompt_cli"
path = "src/lib.rs"

[[bin]]
name = "teleprompt"
path = "src/main.rs"

[dependencies]
teleprompt-compile = { path = "../teleprompt-compile" }
teleprompt-core = { path = "../teleprompt-core" }
teleprompt-scene = { path = "../teleprompt-scene" }
teleprompt-schedule = { path = "../teleprompt-schedule" }
teleprompt-voice = { path = "../teleprompt-voice" }
clap.workspace = true
serde.workspace = true
serde_json.workspace = true
```

- [ ] **Step 4: Implement output formatting and exit codes**

```rust
// crates/teleprompt-cli/src/output.rs
use clap::ValueEnum;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Human,
    Json,
}

/// Every way a command can end. Centralised so no subcommand invents a code.
#[derive(Debug)]
pub enum Outcome {
    Ok,
    RuntimeFailure(String),
    ValidationError(Vec<String>),
    Drift,
    VoiceDowngrade,
}

pub fn exit_code_for(outcome: &Outcome) -> i32 {
    match outcome {
        Outcome::Ok => 0,
        Outcome::RuntimeFailure(_) => 1,
        Outcome::ValidationError(_) => 2,
        Outcome::Drift => 3,
        Outcome::VoiceDowngrade => 4,
    }
}
```

```rust
// crates/teleprompt-cli/src/lib.rs
pub mod cmd;
pub mod output;
```

- [ ] **Step 5: Implement `new`**

```rust
// crates/teleprompt-cli/src/cmd/new.rs
use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};

const PROJECT_TOML: &str = r#"# teleprompt project configuration
[locales]
source = "en"
targets = []

[voice]
source = "synthetic"
backend = "null"

[scene.mock]
adapter = "mock"
"#;

const DEMO_SCRIPT: &str = r#"---
teleprompt: 1
scene:
  mock:
    adapter: mock
---

# Getting started

Welcome to teleprompt. This paragraph is a narration segment, and its spoken
length decides how long the visuals below stay on screen. {#welcome}

```teleprompt scene=mock
wait 500ms
```

Edit that sentence, run `teleprompt diff`, and watch every later transition
move. That feedback loop is the whole point. {#the-loop}

```teleprompt scene=mock policy=concurrent
wait 800ms
```
"#;

const GITIGNORE: &str = ".teleprompt/cache/\n.teleprompt/traces/\nbuild/\ntakes/*.wav\n";

pub fn scaffold(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let manifest = dir.join("teleprompt.toml");
    if manifest.exists() {
        return Err(Error::new(
            ErrorKind::AlreadyExists,
            format!("{} already contains a teleprompt project", dir.display()),
        ));
    }

    std::fs::create_dir_all(dir.join("scripts"))?;
    std::fs::create_dir_all(dir.join("timelines"))?;

    let files = [
        (manifest, PROJECT_TOML),
        (dir.join("scripts/demo.md"), DEMO_SCRIPT),
        (dir.join(".gitignore"), GITIGNORE),
    ];

    let mut written = Vec::new();
    for (path, contents) in files {
        std::fs::write(&path, contents)?;
        written.push(path);
    }
    Ok(written)
}
```

- [ ] **Step 6: Implement `doctor`**

```rust
// crates/teleprompt-cli/src/cmd/doctor.rs
use serde::Serialize;
use teleprompt_scene::SceneRegistry;

#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub ok: bool,
    pub adapters: Vec<String>,
    pub voice_backends: Vec<String>,
    pub notes: Vec<String>,
}

pub fn doctor_report(registry: &SceneRegistry) -> DoctorReport {
    DoctorReport {
        ok: true,
        adapters: registry.available().iter().map(|s| s.to_string()).collect(),
        voice_backends: vec!["null".to_string()],
        notes: vec![
            "M0 builds no video, so ffmpeg is not required yet.".to_string(),
            "M0 ships no external runtime, so Node and Playwright are not required yet."
                .to_string(),
        ],
    }
}

impl DoctorReport {
    pub fn render(&self) -> String {
        let mut out = String::from("teleprompt doctor\n");
        out.push_str(&format!("  scene adapters   {}\n", self.adapters.join(", ")));
        out.push_str(&format!("  voice backends   {}\n", self.voice_backends.join(", ")));
        for n in &self.notes {
            out.push_str(&format!("  note             {n}\n"));
        }
        out
    }
}
```

```rust
// crates/teleprompt-cli/src/cmd/mod.rs
pub mod doctor;
pub mod new;
```

- [ ] **Step 7: Implement the binary entry point**

```rust
// crates/teleprompt-cli/src/main.rs
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use teleprompt_cli::cmd::{doctor, new};
use teleprompt_cli::output::Format;
use teleprompt_scene::SceneRegistry;

#[derive(Parser)]
#[command(name = "teleprompt", version, about = "Compile videos from version-controlled scripts")]
struct Cli {
    #[arg(long, value_enum, global = true, default_value = "human")]
    format: Format,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scaffold a new project
    New { path: PathBuf },
    /// Report the environment teleprompt can see
    Doctor,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let registry = SceneRegistry::with_builtins();

    match cli.command {
        Command::New { path } => match new::scaffold(&path) {
            Ok(files) => {
                match cli.format {
                    Format::Json => println!(
                        "{}",
                        serde_json::json!({
                            "created": files.iter().map(|p| p.display().to_string()).collect::<Vec<_>>()
                        })
                    ),
                    Format::Human => {
                        for f in files {
                            println!("created {}", f.display());
                        }
                    }
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::from(1)
            }
        },
        Command::Doctor => {
            let report = doctor::doctor_report(&registry);
            match cli.format {
                Format::Json => println!("{}", serde_json::to_string_pretty(&report).unwrap()),
                Format::Human => print!("{}", report.render()),
            }
            ExitCode::SUCCESS
        }
    }
}
```

- [ ] **Step 8: Run tests to verify they pass**

Run: `cargo test -p teleprompt-cli`
Expected: PASS, 6 tests.

Run: `cargo run -p teleprompt-cli -- doctor`
Expected: prints the adapter and backend lists.

- [ ] **Step 9: Commit**

```bash
git add crates/teleprompt-cli
git commit -m "feat(cli): binary skeleton with new and doctor"
```

---

### Task 14: `check`, `plan`, and `diff` commands

**Files:**
- Create: `crates/teleprompt-cli/src/cmd/check.rs`, `src/cmd/plan.rs`, `src/cmd/diff.rs`, `src/project.rs`
- Modify: `crates/teleprompt-cli/src/main.rs`, `src/cmd/mod.rs`, `src/lib.rs`
- Test: `crates/teleprompt-cli/tests/commands.rs`

**Interfaces:**
- Consumes: everything from Tasks 1–13.
- Produces:

```rust
pub struct Project { pub root: PathBuf, pub config: PartialConfig }
impl Project { pub fn discover(from: &Path) -> std::io::Result<Project>; pub fn timeline_path(&self, script: &str, locale: &str) -> PathBuf; }

pub fn run_check(project: &Project, script: &Path, locale: &str) -> Result<Vec<String>, Vec<String>>;
pub fn run_plan(project: &Project, script: &Path, locale: &str) -> Result<CompileOutput, Vec<String>>;
pub fn run_diff(project: &Project, script: &Path, locale: &str) -> Result<TimelineDiff, Vec<String>>;
```

`run_diff` reads the committed timeline; a missing one means every beat is `added`.

- [ ] **Step 1: Write the failing tests**

```rust
// crates/teleprompt-cli/tests/commands.rs
use std::path::{Path, PathBuf};

use teleprompt_cli::cmd::{check::run_check, diff::run_diff, plan::run_plan};
use teleprompt_cli::project::Project;

const GOOD: &str = r#"---
scene: { mock: { adapter: mock } }
---

# Intro

One two three four five six. {#welcome}

```teleprompt scene=mock
wait 500ms
```
"#;

const BAD_ATTR: &str = "# Intro\n\nOne. {#a polcy=hold}\n";
const BAD_SCENE: &str = "# Intro\n\nOne. {#a}\n\n```teleprompt scene=mock\nclick things\n```\n";

fn project_with(script: &str) -> (Project, PathBuf) {
    let dir = tempdir();
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    let path = dir.join("scripts/test.md");
    std::fs::write(&path, script).unwrap();
    (Project::discover(&dir).unwrap(), path)
}

#[test]
fn check_accepts_a_valid_script() {
    let (p, s) = project_with(GOOD);
    assert!(run_check(&p, &s, "en").is_ok());
}

#[test]
fn check_reports_an_unknown_attribute_key() {
    let (p, s) = project_with(BAD_ATTR);
    let errs = run_check(&p, &s, "en").unwrap_err();
    assert!(errs[0].contains("unknown attribute key `polcy`"));
}

#[test]
fn check_reports_adapter_validation_errors() {
    let (p, s) = project_with(BAD_SCENE);
    let errs = run_check(&p, &s, "en").unwrap_err();
    assert!(errs[0].contains("unknown mock directive"));
}

#[test]
fn check_does_not_write_anything() {
    let (p, s) = project_with(GOOD);
    let before = listing(&p.root);
    run_check(&p, &s, "en").unwrap();
    assert_eq!(listing(&p.root), before, "check must have no side effects");
}

#[test]
fn plan_produces_a_timeline_without_writing_one() {
    let (p, s) = project_with(GOOD);
    let out = run_plan(&p, &s, "en").unwrap();
    assert!(out.timeline.duration_ms > 0);
    assert!(!p.timeline_path("test.md", "en").exists());
}

#[test]
fn diff_against_a_missing_timeline_reports_every_beat_as_added() {
    let (p, s) = project_with(GOOD);
    let d = run_diff(&p, &s, "en").unwrap();
    assert_eq!(d.added.len(), 1);
    assert!(!d.is_empty());
}

#[test]
fn diff_against_an_identical_committed_timeline_is_empty() {
    let (p, s) = project_with(GOOD);
    let out = run_plan(&p, &s, "en").unwrap();
    let dest = p.timeline_path("test.md", "en");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    std::fs::write(&dest, serde_json::to_string_pretty(&out.timeline).unwrap()).unwrap();

    assert!(run_diff(&p, &s, "en").unwrap().is_empty());
}

#[test]
fn diff_detects_an_edited_paragraph() {
    let (p, s) = project_with(GOOD);
    let out = run_plan(&p, &s, "en").unwrap();
    let dest = p.timeline_path("test.md", "en");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    std::fs::write(&dest, serde_json::to_string_pretty(&out.timeline).unwrap()).unwrap();

    std::fs::write(&s, GOOD.replace("One two three four five six.", "One two three.")).unwrap();
    let d = run_diff(&p, &s, "en").unwrap();
    assert_eq!(d.changed.len(), 1);
    assert!(d.shift_ms < 0, "a shorter paragraph shortens the video");
}

#[test]
fn a_malformed_timeline_on_disk_is_an_error_not_a_panic() {
    let (p, s) = project_with(GOOD);
    let dest = p.timeline_path("test.md", "en");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    std::fs::write(&dest, "{ not json").unwrap();
    assert!(run_diff(&p, &s, "en").is_err());
}

fn listing(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let path = e.path();
            if path.is_dir() {
                stack.push(path.clone());
            }
            out.push(path.display().to_string());
        }
    }
    out.sort();
    out
}

fn tempdir() -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "teleprompt-cmd-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p teleprompt-cli --test commands`
Expected: FAIL — `unresolved import teleprompt_cli::project`.

- [ ] **Step 3: Implement project discovery**

```rust
// crates/teleprompt-cli/src/project.rs
use std::path::{Path, PathBuf};

use teleprompt_core::config::PartialConfig;

pub struct Project {
    pub root: PathBuf,
    pub config: PartialConfig,
}

impl Project {
    /// Walks up from `from` looking for `teleprompt.toml`.
    pub fn discover(from: &Path) -> std::io::Result<Project> {
        let mut dir = from.canonicalize()?;
        loop {
            let candidate = dir.join("teleprompt.toml");
            if candidate.exists() {
                let text = std::fs::read_to_string(&candidate)?;
                let config = PartialConfig::from_toml(&text).map_err(|e| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
                })?;
                return Ok(Project { root: dir, config });
            }
            if !dir.pop() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "no teleprompt.toml found in this directory or any parent",
                ));
            }
        }
    }

    pub fn timeline_path(&self, script: &str, locale: &str) -> PathBuf {
        let stem = Path::new(script)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| script.to_string());
        self.root.join("timelines").join(format!("{stem}.{locale}.json"))
    }
}
```

- [ ] **Step 4: Implement `check` and `plan`**

```rust
// crates/teleprompt-cli/src/cmd/check.rs
use std::path::Path;

use teleprompt_compile::{compile, CompileOutput};
use teleprompt_core::config::PartialConfig;
use teleprompt_core::ident::assign_ids;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::resolve;
use teleprompt_scene::SceneRegistry;
use teleprompt_voice::NullVoice;

use crate::project::Project;

/// Shared front half of every command: read, parse, identify, resolve, compile.
pub fn compile_script(
    project: &Project,
    script: &Path,
    locale: &str,
) -> Result<CompileOutput, Vec<String>> {
    let name = script
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| script.display().to_string());
    let display = script.display().to_string();

    let src = std::fs::read_to_string(script)
        .map_err(|e| vec![format!("cannot read {display}: {e}")])?;

    let mut parsed = parse_script(&src).map_err(|d| render(&d, &display))?;

    let id_diags = assign_ids(&mut parsed);
    if id_diags.iter().any(|d| d.is_error()) {
        return Err(render(&teleprompt_core::Diagnostics(id_diags), &display));
    }

    let program = resolve(&parsed, &name, locale, &project.config, &PartialConfig::default())
        .map_err(|d| render(&d, &display))?;

    compile(&program, &SceneRegistry::with_builtins(), &NullVoice::default(), env!("CARGO_PKG_VERSION"))
        .map_err(|d| render(&d, &display))
}

/// Returns warnings on success, rendered errors on failure.
pub fn run_check(project: &Project, script: &Path, locale: &str) -> Result<Vec<String>, Vec<String>> {
    compile_script(project, script, locale).map(|out| out.warnings)
}

fn render(d: &teleprompt_core::Diagnostics, file: &str) -> Vec<String> {
    d.0.iter().map(|x| x.render(file)).collect()
}
```

```rust
// crates/teleprompt-cli/src/cmd/plan.rs
use std::path::Path;

use teleprompt_compile::CompileOutput;

use crate::cmd::check::compile_script;
use crate::project::Project;

pub fn run_plan(project: &Project, script: &Path, locale: &str) -> Result<CompileOutput, Vec<String>> {
    compile_script(project, script, locale)
}

pub fn render_plan(out: &CompileOutput) -> String {
    let mut s = format!(
        "{} ({}) — {:.1}s across {} beat(s)\n",
        out.timeline.script,
        out.timeline.locale,
        out.timeline.duration_ms as f64 / 1000.0,
        out.timeline.entries.len()
    );
    for e in &out.timeline.entries {
        let narration = e
            .narration
            .as_ref()
            .map(|n| format!("{:.1}s {}", n.duration_ms as f64 / 1000.0, n.segment))
            .unwrap_or_else(|| "—".to_string());
        s.push_str(&format!(
            "  {:>8.1}s  {:<10} {}\n",
            e.start_ms as f64 / 1000.0,
            e.policy,
            narration
        ));
    }
    for w in &out.warnings {
        s.push_str(&format!("  warning: {w}\n"));
    }
    s
}
```

- [ ] **Step 5: Implement `diff`**

```rust
// crates/teleprompt-cli/src/cmd/diff.rs
use std::path::Path;

use teleprompt_schedule::{diff, Timeline, TimelineDiff};

use crate::cmd::plan::run_plan;
use crate::project::Project;

pub fn run_diff(project: &Project, script: &Path, locale: &str) -> Result<TimelineDiff, Vec<String>> {
    let out = run_plan(project, script, locale)?;
    let name = script
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let committed_path = project.timeline_path(&name, locale);

    let committed = if committed_path.exists() {
        let text = std::fs::read_to_string(&committed_path)
            .map_err(|e| vec![format!("cannot read {}: {e}", committed_path.display())])?;
        serde_json::from_str::<Timeline>(&text).map_err(|e| {
            vec![format!("{} is not a valid timeline: {e}", committed_path.display())]
        })?
    } else {
        Timeline {
            version: teleprompt_schedule::TIMELINE_VERSION,
            script: out.timeline.script.clone(),
            locale: out.timeline.locale.clone(),
            duration_ms: 0,
            generated_by: String::new(),
            entries: Vec::new(),
        }
    };

    Ok(diff(&committed, &out.timeline))
}
```

```rust
// crates/teleprompt-cli/src/cmd/mod.rs — replace
pub mod check;
pub mod diff;
pub mod doctor;
pub mod new;
pub mod plan;
```

```rust
// crates/teleprompt-cli/src/lib.rs — replace
pub mod cmd;
pub mod output;
pub mod project;
```

- [ ] **Step 6: Wire the subcommands into the binary**

Add to `Command` in `main.rs`:

```rust
    /// Parse and validate; no side effects, no cost
    Check {
        script: PathBuf,
        #[arg(long, default_value = "en")]
        locale: String,
    },
    /// Compile the timeline and print it
    Plan {
        script: PathBuf,
        #[arg(long, default_value = "en")]
        locale: String,
    },
    /// Compare against the committed timeline
    Diff {
        script: PathBuf,
        #[arg(long, default_value = "en")]
        locale: String,
        /// Exit 3 when the timeline has drifted
        #[arg(long)]
        exit_code: bool,
    },
```

And the match arms:

```rust
        Command::Check { script, locale } => {
            let Ok(project) = teleprompt_cli::project::Project::discover(
                script.parent().unwrap_or(std::path::Path::new(".")),
            ) else {
                eprintln!("error: no teleprompt.toml found");
                return ExitCode::from(1);
            };
            match teleprompt_cli::cmd::check::run_check(&project, &script, &locale) {
                Ok(warnings) => {
                    for w in &warnings {
                        eprintln!("warning: {w}");
                    }
                    if matches!(cli.format, Format::Json) {
                        println!("{}", serde_json::json!({ "ok": true, "warnings": warnings }));
                    } else {
                        println!("ok");
                    }
                    ExitCode::SUCCESS
                }
                Err(errors) => {
                    if matches!(cli.format, Format::Json) {
                        println!("{}", serde_json::json!({ "ok": false, "errors": errors }));
                    } else {
                        for e in &errors {
                            eprintln!("{e}");
                        }
                    }
                    ExitCode::from(2)
                }
            }
        }
        Command::Plan { script, locale } => {
            let Ok(project) = teleprompt_cli::project::Project::discover(
                script.parent().unwrap_or(std::path::Path::new(".")),
            ) else {
                eprintln!("error: no teleprompt.toml found");
                return ExitCode::from(1);
            };
            match teleprompt_cli::cmd::plan::run_plan(&project, &script, &locale) {
                Ok(out) => {
                    match cli.format {
                        Format::Json => {
                            println!("{}", serde_json::to_string_pretty(&out.timeline).unwrap())
                        }
                        Format::Human => print!("{}", teleprompt_cli::cmd::plan::render_plan(&out)),
                    }
                    ExitCode::SUCCESS
                }
                Err(errors) => {
                    for e in &errors {
                        eprintln!("{e}");
                    }
                    ExitCode::from(2)
                }
            }
        }
        Command::Diff { script, locale, exit_code } => {
            let Ok(project) = teleprompt_cli::project::Project::discover(
                script.parent().unwrap_or(std::path::Path::new(".")),
            ) else {
                eprintln!("error: no teleprompt.toml found");
                return ExitCode::from(1);
            };
            match teleprompt_cli::cmd::diff::run_diff(&project, &script, &locale) {
                Ok(d) => {
                    match cli.format {
                        Format::Json => println!("{}", serde_json::to_string_pretty(&d).unwrap()),
                        Format::Human => println!("{}", d.render()),
                    }
                    if exit_code && !d.is_empty() {
                        ExitCode::from(3)
                    } else {
                        ExitCode::SUCCESS
                    }
                }
                Err(errors) => {
                    for e in &errors {
                        eprintln!("{e}");
                    }
                    ExitCode::from(2)
                }
            }
        }
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test --workspace`
Expected: PASS, 9 new tests; whole workspace green.

- [ ] **Step 8: Commit**

```bash
git add crates/teleprompt-cli
git commit -m "feat(cli): check, plan, and diff commands"
```

---

### Task 15: End-to-end acceptance and CI

**Files:**
- Create: `tests/fixtures/tour.md`
- Create: `crates/teleprompt-cli/tests/end_to_end.rs`
- Create: `.github/workflows/ci.yml`
- Create: `README.md`
- Test: the acceptance suite itself

This task answers M0's question — *does editing prose produce a legible, useful diff of the video's pacing?* — against a realistic multi-chapter script, and locks the answer in with CI.

**Interfaces:**
- Consumes: the whole workspace.
- Produces: no new library API. The fixture and the acceptance suite are the deliverable.

- [ ] **Step 1: Write the fixture script**

```markdown
<!-- tests/fixtures/tour.md -->
---
teleprompt: 1
locales:
  source: en
scene:
  mock:
    adapter: mock
output:
  transition: { duration: auto, max_ms: 600 }
---

# Introduction

Welcome to Acme. In the next two minutes I will show you how to get a project
running, deploy it, and roll it back when something goes wrong. {#welcome}

```teleprompt scene=mock
wait 400ms
```

Everything you see here is generated from a single Markdown file that lives in
version control alongside the code it documents. {#provenance}

```teleprompt scene=mock policy=concurrent align=start
wait 1200ms
mark
wait 600ms
```

# Deploying

Deployment is one command, and it streams progress as it goes. {#deploy}

```teleprompt scene=mock policy=stretch
wait 900ms
```

<!-- teleprompt: pause 600ms -->

If a deploy goes wrong, rolling back takes the same single command with one
extra flag. {#rollback}

```teleprompt scene=mock policy=trim
wait 3000ms
```
```

- [ ] **Step 2: Write the failing acceptance tests**

```rust
// crates/teleprompt-cli/tests/end_to_end.rs
use std::path::PathBuf;

use teleprompt_cli::cmd::{check::run_check, diff::run_diff, plan::run_plan};
use teleprompt_cli::project::Project;

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    std::fs::read_to_string(path).expect("fixture must exist")
}

fn workspace() -> (Project, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "teleprompt-e2e-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    let script = dir.join("scripts/tour.md");
    std::fs::write(&script, fixture("tour.md")).unwrap();
    (Project::discover(&dir).unwrap(), script)
}

#[test]
fn the_full_fixture_validates() {
    let (p, s) = workspace();
    let warnings = run_check(&p, &s, "en").expect("fixture must be valid");
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
}

#[test]
fn every_policy_appears_in_the_compiled_timeline() {
    let (p, s) = workspace();
    let out = run_plan(&p, &s, "en").unwrap();
    let policies: std::collections::BTreeSet<&str> =
        out.timeline.entries.iter().map(|e| e.policy.as_str()).collect();
    for expected in ["hold", "concurrent", "stretch", "trim"] {
        assert!(policies.contains(expected), "missing policy {expected}");
    }
}

#[test]
fn beats_never_overlap_and_never_gap() {
    let (p, s) = workspace();
    let out = run_plan(&p, &s, "en").unwrap();
    for pair in out.timeline.entries.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let expected = a.start_ms + a.duration_ms - a.transition.duration_ms;
        assert_eq!(b.start_ms, expected, "gap or overlap between {} and {}", a.beat, b.beat);
    }
}

#[test]
fn the_timeline_ends_where_the_last_beat_ends() {
    let (p, s) = workspace();
    let out = run_plan(&p, &s, "en").unwrap();
    let last = out.timeline.entries.last().unwrap();
    assert_eq!(out.timeline.duration_ms, last.start_ms + last.duration_ms);
}

#[test]
fn planning_twice_gives_byte_identical_output() {
    let (p, s) = workspace();
    let a = serde_json::to_string(&run_plan(&p, &s, "en").unwrap().timeline).unwrap();
    let b = serde_json::to_string(&run_plan(&p, &s, "en").unwrap().timeline).unwrap();
    assert_eq!(a, b);
}

/// The M0 acceptance criterion.
#[test]
fn editing_one_paragraph_shows_up_as_a_legible_pacing_diff() {
    let (p, s) = workspace();

    let baseline = run_plan(&p, &s, "en").unwrap();
    let dest = p.timeline_path("tour.md", "en");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    std::fs::write(&dest, serde_json::to_string_pretty(&baseline.timeline).unwrap()).unwrap();

    let edited = fixture("tour.md").replace(
        "Deployment is one command, and it streams progress as it goes.",
        "Deployment is one single command, and it streams its progress as it goes along, \
         step by step, so you always know exactly where you are.",
    );
    std::fs::write(&s, edited).unwrap();

    let d = run_diff(&p, &s, "en").unwrap();

    assert_eq!(d.changed.len(), 1, "exactly one segment changed");
    assert_eq!(d.changed[0].beat, "deploy");
    assert!(d.changed[0].reason.contains("text edited"));
    assert!(d.shift_ms > 0, "a longer paragraph lengthens the video");
    assert!(!d.recapture.is_empty(), "the stretched beat needs re-capture");

    let rendered = d.render();
    assert!(rendered.contains("deploy"));
    assert!(rendered.contains('→'));
}

#[test]
fn an_unedited_script_diffs_clean_against_its_committed_timeline() {
    let (p, s) = workspace();
    let out = run_plan(&p, &s, "en").unwrap();
    let dest = p.timeline_path("tour.md", "en");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    std::fs::write(&dest, serde_json::to_string_pretty(&out.timeline).unwrap()).unwrap();

    let d = run_diff(&p, &s, "en").unwrap();
    assert!(d.is_empty(), "clean checkout must diff clean: {}", d.render());
}
```

- [ ] **Step 3: Run the acceptance suite to verify it fails**

Run: `cargo test -p teleprompt-cli --test end_to_end`
Expected: FAIL — the fixture file does not exist yet if Step 1 was skipped; otherwise failures point at real gaps.

- [ ] **Step 4: Fix whatever the suite exposes**

Do not weaken an assertion to make it pass. Each one encodes a spec requirement:
- overlapping/gapping beats means the transition arithmetic in `schedule()` is wrong;
- a missing policy means `Policy::parse` or the fence attribute plumbing dropped it;
- a non-deterministic plan means something iterates a `HashMap` — switch it to `BTreeMap`.

- [ ] **Step 5: Run the whole workspace**

Run: `cargo test --workspace`
Expected: PASS, all suites.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: no warnings.

Run: `cargo fmt --all --check`
Expected: no diff.

- [ ] **Step 6: Add CI**

```yaml
# .github/workflows/ci.yml
name: CI

on:
  push:
    branches: [main]
  pull_request:

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.75
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --all --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace
```

M0 needs no services, no browser, and no ffmpeg, so CI is a plain checkout and three commands. Keeping it that way is a design constraint, not an accident.

- [ ] **Step 7: Write the README**

```markdown
# teleprompt

Compile narrated videos from version-controlled Markdown.

A script's prose is its narration; fenced `teleprompt` blocks are its visuals.
Narration duration drives visual pacing, so editing a paragraph changes the
rhythm of the video — and `teleprompt diff` tells you exactly how before you
render anything.

**Status: M0.** The compiler and the feedback loop work end to end. There is no
video output yet — see `docs/superpowers/specs/2026-08-15-teleprompt-design.md`
for the full design and milestones.

## Try it

```bash
cargo run -- new demo
cargo run -- plan demo/scripts/demo.md
cargo run -- diff demo/scripts/demo.md
```

Edit a paragraph in `demo/scripts/demo.md`, run `diff` again, and watch the
transitions move.

## Commands

| command | does |
|---|---|
| `new <path>` | scaffold a project |
| `check <script>` | parse and validate; no side effects, no cost |
| `plan <script>` | compile the timeline and print it |
| `diff <script>` | compare against the committed timeline |
| `doctor` | report the environment teleprompt can see |

All commands accept `--format json`.
```

- [ ] **Step 8: Commit**

```bash
git add tests/fixtures crates/teleprompt-cli/tests/end_to_end.rs .github/workflows/ci.yml README.md
git commit -m "test: end-to-end acceptance for the M0 feedback loop"
```

---

### Task 16: Chapter front matter and `include=`

**Files:**
- Modify: `crates/teleprompt-core/src/ast.rs`, `src/parse.rs`, `src/program.rs`
- Modify: `crates/teleprompt-compile/src/lib.rs`
- Test: `crates/teleprompt-core/tests/chapter_config.rs`, `crates/teleprompt-compile/tests/include.rs`

Two spec requirements the earlier tasks do not cover: configuration layer 4 of §3.5 (a YAML block immediately after a heading) and the `include=` fence attribute from §3.2.

`include=` resolves in `teleprompt-compile`, not `teleprompt-core`, because reading a second file is filesystem access and core's purity constraint forbids it. Compile already does IO, so that is where the path is read and substituted into the `BlockSource` before validation.

**Interfaces:**
- Consumes: `Chapter` (Task 2), `resolve` (Task 8), `compile` (Task 11).
- Produces: `Chapter.front_matter: String` (empty when absent); `compile` gains a `base_dir: &Path` parameter used to resolve `include=` relative to the script.

- [ ] **Step 1: Write the failing tests**

```rust
// crates/teleprompt-core/tests/chapter_config.rs
use teleprompt_core::config::PartialConfig;
use teleprompt_core::ident::assign_ids;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::{resolve, Item};

const SRC: &str = r#"---
timing:
  lead_in_ms: 100
---

# Fast

One.

# Slow

```yaml teleprompt
timing:
  lead_in_ms: 900
```

Two.
"#;

fn items() -> Vec<Item> {
    let mut s = parse_script(SRC).unwrap();
    assign_ids(&mut s);
    resolve(&s, "d.md", "en", &PartialConfig::default(), &PartialConfig::default())
        .unwrap()
        .items
}

#[test]
fn chapter_front_matter_overrides_script_front_matter() {
    let it = items();
    let Item::Narration { config: fast, .. } = &it[0] else { panic!() };
    let Item::Narration { config: slow, .. } = &it[1] else { panic!() };
    assert_eq!(fast.timing.lead_in_ms, 100);
    assert_eq!(slow.timing.lead_in_ms, 900);
}

#[test]
fn a_chapter_config_block_is_not_narration() {
    assert_eq!(items().len(), 2, "the yaml block must not become a segment");
}

#[test]
fn chapter_front_matter_is_optional() {
    let mut s = parse_script("# A\n\nOne.\n").unwrap();
    assign_ids(&mut s);
    assert!(s.chapters[0].front_matter.is_empty());
}

#[test]
fn malformed_chapter_front_matter_is_a_diagnostic() {
    let src = "# A\n\n```yaml teleprompt\ntiming: [nope\n```\n\nOne.\n";
    let mut s = parse_script(src).unwrap();
    assign_ids(&mut s);
    let e = resolve(&s, "d.md", "en", &PartialConfig::default(), &PartialConfig::default())
        .unwrap_err();
    assert!(e.0[0].message.contains("chapter `a` front matter"));
}
```

```rust
// crates/teleprompt-compile/tests/include.rs
use std::path::PathBuf;

use teleprompt_compile::compile;
use teleprompt_core::config::PartialConfig;
use teleprompt_core::ident::assign_ids;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::resolve;
use teleprompt_scene::SceneRegistry;
use teleprompt_voice::NullVoice;

fn workspace() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "teleprompt-include-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &PathBuf, src: &str) -> Result<teleprompt_compile::CompileOutput, Vec<String>> {
    let mut s = parse_script(src).unwrap();
    assign_ids(&mut s);
    let p = resolve(&s, "d.md", "en", &PartialConfig::default(), &PartialConfig::default())
        .unwrap();
    compile(&p, &SceneRegistry::with_builtins(), &NullVoice::default(), dir, "0.1.0")
        .map_err(|d| d.0.iter().map(|x| x.message.clone()).collect())
}

#[test]
fn an_included_file_supplies_the_block_body() {
    let dir = workspace();
    std::fs::write(dir.join("steps.mock"), "wait 700ms\n").unwrap();
    let out = run(&dir, "# A\n\nOne. {#a}\n\n```teleprompt scene=mock include=steps.mock\n```\n")
        .unwrap();
    assert_eq!(out.timeline.entries[0].action.as_ref().unwrap().duration_ms, 700);
}

#[test]
fn a_missing_include_is_a_diagnostic() {
    let dir = workspace();
    let e = run(&dir, "# A\n\nOne. {#a}\n\n```teleprompt scene=mock include=absent.mock\n```\n")
        .unwrap_err();
    assert!(e[0].contains("cannot read included file `absent.mock`"));
}

#[test]
fn a_fence_with_both_a_body_and_an_include_is_an_error() {
    let dir = workspace();
    std::fs::write(dir.join("steps.mock"), "wait 700ms\n").unwrap();
    let e = run(
        &dir,
        "# A\n\nOne. {#a}\n\n```teleprompt scene=mock include=steps.mock\nwait 1s\n```\n",
    )
    .unwrap_err();
    assert!(e[0].contains("has both a body and an `include`"));
}

#[test]
fn an_include_escaping_the_project_root_is_refused() {
    let dir = workspace();
    let e = run(&dir, "# A\n\nOne. {#a}\n\n```teleprompt scene=mock include=../outside.mock\n```\n")
        .unwrap_err();
    assert!(e[0].contains("outside the project"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p teleprompt-core --test chapter_config -p teleprompt-compile --test include`
Expected: FAIL — `Chapter` has no field `front_matter`; `compile` takes four arguments, not five.

- [ ] **Step 3: Parse chapter front matter**

Add `pub front_matter: String` to `Chapter` in `ast.rs`, initialised to `String::new()`.

In `parse.rs`, a fenced block whose info string is `yaml teleprompt` and which is the first node of a chapter becomes that chapter's front matter rather than a node:

```rust
// in parse_body(), inside Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info)))
fence_info = info.to_string();
let words: Vec<&str> = fence_info.split_whitespace().collect();
state = match words.as_slice() {
    ["yaml", "teleprompt"] => State::ChapterConfig,
    [FENCE_TAG, ..] => State::ActionBlock,
    _ => State::Idle,
};
text.clear();
```

```rust
// in Event::End(TagEnd::CodeBlock), before the ActionBlock arm
if matches!(state, State::ChapterConfig) {
    match chapters.last_mut() {
        Some(ch) if ch.nodes.is_empty() => ch.front_matter = text.clone(),
        Some(_) => diags.push(
            Diagnostic::error("chapter configuration must come directly after the heading")
                .at(span),
        ),
        None => diags.push(
            Diagnostic::error("chapter configuration appears before the first heading").at(span),
        ),
    }
    state = State::Idle;
    text.clear();
    continue;
}
```

Add `ChapterConfig` to the `State` enum. Note that `FENCE_TAG` cannot be used in a slice pattern directly — match on `words.first()` and `words.get(1)` instead if the compiler objects:

```rust
state = if words.first() == Some(&"yaml") && words.get(1) == Some(&FENCE_TAG) {
    State::ChapterConfig
} else if words.first() == Some(&FENCE_TAG) {
    State::ActionBlock
} else {
    State::Idle
};
```

- [ ] **Step 4: Add the chapter layer to resolution**

In `program.rs`, parse each chapter's front matter once per chapter and slot it between the script front matter and the attributes:

```rust
    for chapter in &script.chapters {
        let chapter_cfg = match PartialConfig::from_yaml(&chapter.front_matter) {
            Ok(c) => c,
            Err(e) => {
                diags.push(Diagnostic::error(format!(
                    "chapter `{}` front matter: {e}",
                    chapter.slug
                )));
                PartialConfig::default()
            }
        };
        // every Config::merged call in this loop becomes:
        //   &[project.clone(), front.clone(), chapter_cfg.clone(),
        //     PartialConfig::from_attrs(&attrs), cli.clone()]
```

- [ ] **Step 5: Resolve `include=` in compile**

Change `compile`'s signature to take `base_dir: &Path` and substitute the body before validation:

```rust
// in the Item::Action arm, before building BlockSource
let body = match include {
    Some(rel) => {
        let path = base_dir.join(rel);
        if !path.starts_with(base_dir) {
            diags.push(Diagnostic::error(format!(
                "included file `{rel}` resolves outside the project"
            )));
            continue;
        }
        if !body.trim().is_empty() {
            diags.push(Diagnostic::error(format!(
                "action block has both a body and an `include={rel}`"
            )));
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                diags.push(Diagnostic::error(format!(
                    "cannot read included file `{rel}`: {e}"
                )));
                continue;
            }
        }
    }
    None => body.clone(),
};
```

`Item::Action` gains `pub include: Option<String>`, populated in `program.rs` from `attrs.get("include")`.

The `starts_with` check runs against the *joined* path before canonicalisation, so `../` is caught literally. That is deliberate: canonicalising first would follow symlinks out of the project and pass the check.

Update the four existing `compile(...)` call sites — three in `crates/teleprompt-compile/tests/compile.rs` (via its `run` helper) and one in `crates/teleprompt-cli/src/cmd/check.rs` — to pass the script's parent directory.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --workspace`
Expected: PASS, 8 new tests; whole workspace green.

- [ ] **Step 7: Commit**

```bash
git add crates
git commit -m "feat(core): chapter-level configuration and include= action blocks"
```

---

## Definition of done

M0 is complete when all of these hold:

- [ ] `cargo test --workspace` passes with no ignored tests.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` is clean.
- [ ] `cargo fmt --all --check` is clean.
- [ ] The workspace builds with no network, no browser, no Node, and no ffmpeg.
- [ ] `teleprompt new`, `check`, `plan`, `diff`, and `doctor` all work against the scaffolded project.
- [ ] Editing one paragraph of `tests/fixtures/tour.md` produces a diff naming that segment, its old and new duration, and the total shift.
- [ ] Planning the same script twice produces byte-identical JSON.
- [ ] `teleprompt-core` and `teleprompt-schedule` depend on no async runtime and no network crate.

## What M0 deliberately leaves out

Each of these is specified and scheduled, and none is a gap in this plan:

| omitted | milestone |
|---|---|
| Any video output, ffmpeg, compositing | M1 |
| Real TTS (`kokoro`, `elevenlabs`) | M1, M3 |
| The caching store | M1 |
| Playwright adapter, Node sidecar, `tp` helper, measurement pass | M2 |
| Voice cloning, `voice enroll`, `--strict-voice` | M3 |
| Translation sidecars, `loc sync|status`, per-locale builds | M3 |
| The prompter, take manifest and slices, staleness | M4 |
| `import`, traces, ASR, policy inference | M5 |
| VHS adapter, `serve` | M6 |

The `SceneCompiler` contract, the `VoiceBackend` contract, the fallback ladder,
and the `source_hash` binding all exist from M0 even though their real
implementations arrive later — those are the pieces that are expensive to
retrofit.
