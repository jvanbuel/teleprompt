//! `import`'s drafting step, at the seam it was agreed at: Markdown in,
//! teleprompt script out, no filesystem.

use teleprompt_draft::derive::document::draft;

#[test]
fn a_paragraph_becomes_a_line_carrying_its_own_id() {
    // A derived id shifts when a paragraph is inserted above it, which silently
    // invalidates caches and takes. A drafted script promotes the id at birth
    // so the first edit cannot move it.
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
    assert!(out.contains("```teleprompt scene=vhs"), "{out}");
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
    assert!(out.contains("scene=vhs review=pending"), "{out}");
}

#[test]
fn a_non_shell_fence_survives_as_ordinary_markdown() {
    let out = draft("Here is the payload.\n\n```json\n{\"a\": 1}\n```\n", "Acme");
    assert!(out.contains("```json\n{\"a\": 1}\n```"), "{out}");
    assert!(!out.contains("scene=vhs"), "{out}");
}

#[test]
fn two_paragraphs_that_open_the_same_way_get_different_ids() {
    // Ids are the anchor for caching and the manifest's audio files. Two of
    // them colliding would make two lines one.
    let out = draft(
        "Run the build. It takes a while.\n\nRun the build again.\n",
        "Acme",
    );
    assert!(out.contains("{#run-the-build}"), "{out}");
    assert!(out.contains("{#run-the-build-2}"), "{out}");
}

mod slidev {
    use teleprompt_draft::derive::document::draft_slidev;

    const DECK: &str = "---
theme: default
title: Narrated slides
layout: cover
---

# Narrated slides

<!-- This is a deck, narrated. -->

---

# Your deck stays a deck

<v-clicks>

- one
- two

</v-clicks>

<!--
The deck does not change.
[click] You can still present it.
[click:2] You can still export it.
-->

---
layout: center
---

# No notes here

---

```md
---
not a separator
---
<!-- not a note either -->
```

<!-- A slide whose code block holds a separator. -->
";

    #[test]
    fn notes_become_paragraphs_and_clicks_become_steps() {
        let d = draft_slidev(DECK, "deck/slides.md", &|_| None);
        let s = &d.script;
        assert!(
            s.contains("scene:\n  slidev:\n    deck: deck/slides.md\n"),
            "{s}"
        );
        assert!(s.contains("# Narrated slides\n\nThis is a deck, narrated. {#this-is-a}\n\n```teleprompt scene=slidev policy=concurrent\n1\n```"), "{s}");
        assert!(
            s.contains("The deck does not change. {#the-deck-does}"),
            "{s}"
        );
        assert!(s.contains("You can still present it. {#you-can-still}\n\n```teleprompt scene=slidev policy=concurrent\n2?clicks=1\n```"), "{s}");
        // `[click:2]` adds two, as it does in Slidev.
        assert!(s.contains("You can still export it. {#you-can-still-2}\n\n```teleprompt scene=slidev policy=concurrent\n2?clicks=3\n```"), "{s}");
    }

    /// A slide with nothing to say is left out and reported, since a block
    /// with no sentence would last no time at all.
    #[test]
    fn a_slide_without_notes_is_left_out_and_said_so() {
        let d = draft_slidev(DECK, "slides.md", &|_| None);
        assert_eq!(d.silent, vec![3]);
        assert!(!d.script.contains("No notes here"));
    }

    /// A slide that is only a comment's ends, `<!-->`, has no note.
    #[test]
    fn a_slide_of_a_bare_comment_has_no_note() {
        let d = draft_slidev(
            "# One\n\n<!-- Said. -->\n\n---\n\n<!-->\n",
            "slides.md",
            &|_| None,
        );
        assert_eq!(d.silent, vec![2]);
    }

    /// A `---` inside a code fence is not a slide break, and a comment
    /// inside one is not a note — Slidev's parser skips fences, and so
    /// does this.
    #[test]
    fn code_fences_hide_separators_and_comments() {
        let d = draft_slidev(DECK, "slides.md", &|_| None);
        assert!(d
            .script
            .contains("A slide whose code block holds a separator."));
        assert!(
            d.script.contains("\n4\n```"),
            "the fourth slide is slide 4: {}",
            d.script
        );
        assert!(!d.script.contains("not a note"));
    }

    /// Frontmatter `title:` names the chapter; otherwise the first heading.
    #[test]
    fn a_slide_is_titled_like_slidev_titles_it() {
        let d = draft_slidev(DECK, "slides.md", &|_| None);
        assert!(d.script.contains("# Your deck stays a deck\n"));
        assert!(
            d.script.contains("# Slide 4\n"),
            "no heading, no title: {}",
            d.script
        );
    }

    /// The slide number each paragraph's block names, in order.
    fn numbers(script: &str) -> Vec<String> {
        script
            .split("```teleprompt scene=slidev policy=concurrent\n")
            .skip(1)
            .map(|b| b.lines().next().unwrap_or("").to_string())
            .collect()
    }

    /// Slidev drops a hidden or disabled slide from its numbering — found
    /// by exporting a deck whose fourth slide came out as slide 2 — so the
    /// slides after one are numbered as if it were not there.
    #[test]
    fn hidden_and_disabled_slides_have_no_number() {
        let deck = "# One\n\n<!-- one -->\n\n---\nhide: true\n---\n\n# Two\n\n<!-- two -->\n\n\
                    ---\ndisabled: true\n---\n\n# Three\n\n---\n\n# Four\n\n<!-- four -->\n";
        let d = draft_slidev(deck, "slides.md", &|_| None);
        assert_eq!(numbers(&d.script), ["1", "2"], "{}", d.script);
        assert!(d
            .script
            .contains("four {#four}\n\n```teleprompt scene=slidev policy=concurrent\n2\n"));
        assert!(!d.script.contains("two {#two}"));
        assert_eq!(
            d.warnings.len(),
            1,
            "a hidden slide's notes are dropped, and said so: {:?}",
            d.warnings
        );
        assert!(
            d.warnings[0].contains("`Two` is hidden"),
            "{:?}",
            d.warnings
        );
    }

    /// `src:` contributes every slide of the file it names, relative to the
    /// importing file, and `#2-3` selects some of them.
    #[test]
    fn an_import_contributes_its_slides_to_the_numbering() {
        let deck = "# One\n\n<!-- one -->\n\n---\nsrc: ./pages/part.md#2-3\n---\n\n---\n\n# Last\n\n<!-- last -->\n";
        let part =
            "# A\n\n<!-- a -->\n\n---\n\n# B\n\n<!-- b -->\n\n---\nsrc: ../pages/inner.md\n---\n";
        let inner = "# C\n\n<!-- c -->\n";
        let read = |path: &str| match path {
            "pages/part.md" => Some(part.to_string()),
            "pages/inner.md" => Some(inner.to_string()),
            _ => None,
        };
        let d = draft_slidev(deck, "slides.md", &read);
        // one, then part's slides 2 (B) and 3 (the import of inner: C), then last.
        assert_eq!(numbers(&d.script), ["1", "2", "3", "4"], "{}", d.script);
        assert!(d.script.contains("# B\n"));
        assert!(d.script.contains("# C\n"));
        assert!(!d.script.contains("# A\n"), "outside the range");
        assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    }

    /// An import that cannot be read, or that imports itself, is a warning:
    /// every number after it may be wrong, and the author should know.
    #[test]
    fn an_unreadable_or_circular_import_is_said_so() {
        let deck = "---\nsrc: missing.md\n---\n\n---\nsrc: loop.md\n---\n";
        let read =
            |path: &str| (path == "loop.md").then(|| "---\nsrc: ./loop.md\n---\n".to_string());
        let d = draft_slidev(deck, "slides.md", &read);
        assert_eq!(d.warnings.len(), 2, "{:?}", d.warnings);
        assert!(d.warnings[0].contains("could not be read"));
        assert!(d.warnings[1].contains("imports itself"));
    }
}
