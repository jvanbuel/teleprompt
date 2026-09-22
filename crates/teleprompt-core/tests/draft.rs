//! `from`'s drafting step, at the seam it was agreed at: Markdown in,
//! teleprompt script out, no filesystem.

use teleprompt_core::draft::draft;

#[test]
fn a_paragraph_becomes_a_line_carrying_its_own_id() {
    // §3.3: a derived id shifts when a paragraph is inserted above it, which
    // silently invalidates caches and takes. A drafted script promotes the
    // id at birth so the first edit cannot move it.
    let out = draft("Welcome to Acme. Let me show you around.\n", "Acme");
    assert!(
        out.contains("Welcome to Acme. Let me show you around. {#welcome-to-acme}"),
        "{out}"
    );
}

#[test]
fn a_heading_stays_a_heading_and_is_not_spoken() {
    // Headings are chapters in both formats, so the draft carries them
    // through untouched — an id on a heading would make it narration.
    let out = draft("# Getting started\n\nFirst paragraph.\n", "Acme");
    assert!(out.contains("# Getting started\n"), "{out}");
    assert!(!out.contains("# Getting started {#"), "{out}");
}

#[test]
fn a_shell_fence_becomes_a_tape_that_types_the_command() {
    // The command is typed and never run. A draft that executed what it
    // found in someone's README would be a very sharp edge indeed.
    let out = draft("Install it.\n\n```bash\nnpm install acme\n```\n", "Acme");
    assert!(out.contains("```teleprompt scene=terminal"), "{out}");
    assert!(out.contains("Type \"npm install acme\""), "{out}");
    assert!(out.contains("Enter"), "{out}");
    assert!(
        !out.contains("```bash"),
        "the fence is replaced, not kept: {out}"
    );
}

#[test]
fn a_drafted_tape_is_marked_unreviewed() {
    // A command lifted out of someone's README has not been read by anyone
    // yet. The attribute is the record of that, and `check` is what nags.
    let out = draft("Install it.\n\n```bash\nnpm install acme\n```\n", "Acme");
    assert!(out.contains("scene=terminal review=pending"), "{out}");
}

#[test]
fn a_non_shell_fence_survives_as_ordinary_markdown() {
    let out = draft("Here is the payload.\n\n```json\n{\"a\": 1}\n```\n", "Acme");
    assert!(out.contains("```json\n{\"a\": 1}\n```"), "{out}");
    assert!(!out.contains("scene=terminal"), "{out}");
}

#[test]
fn two_paragraphs_that_open_the_same_way_get_different_ids() {
    // Ids are the anchor for caching, translation and take binding (§3.3).
    // Two of them colliding would make two lines one.
    let out = draft(
        "Run the build. It takes a while.\n\nRun the build again.\n",
        "Acme",
    );
    assert!(out.contains("{#run-the-build}"), "{out}");
    assert!(out.contains("{#run-the-build-2}"), "{out}");
}
