//! Timeline drags as script edits: each changes a block's attributes or
//! its place, and nothing else in the file.

use teleprompt_core::edit::{apply, Edit};

const SCRIPT: &str = "---
teleprompt: 1
---

# Tour

Let's see what is here, and what it does. {#look}

```teleprompt scene=vhs include=recordings/tour.tape#1
```

And now the file. {#file}

```teleprompt scene=vhs include=recordings/tour.tape#2 policy=concurrent cue=\"the file\"
```
";

fn edit(e: Edit) -> String {
    apply(SCRIPT, &e).unwrap_or_else(|err| panic!("{err}"))
}

fn fence(script: &str, n: usize) -> &str {
    script
        .lines()
        .filter(|l| l.starts_with("```teleprompt"))
        .nth(n)
        .unwrap()
}

#[test]
fn a_block_dragged_onto_a_word_is_cued_there() {
    let out = edit(Edit::Cue {
        block: "look-a".into(),
        word: 2,
    });
    assert_eq!(
        fence(&out, 0),
        "```teleprompt scene=vhs include=recordings/tour.tape#1 policy=concurrent cue=\"what is\""
    );
    // Everything else is as it was.
    assert_eq!(
        out.replace(fence(&out, 0), ""),
        SCRIPT.replace(fence(SCRIPT, 0), "")
    );
}

#[test]
fn a_cue_grows_until_the_line_says_it_once() {
    // "what it" is the shortest phrase from the second "what".
    let out = edit(Edit::Cue {
        block: "look-a".into(),
        word: 6,
    });
    assert!(
        fence(&out, 0).ends_with("cue=\"what it\""),
        "{}",
        fence(&out, 0)
    );
}

#[test]
fn the_first_word_starts_with_the_line_without_a_cue() {
    let out = edit(Edit::Cue {
        block: "file-a".into(),
        word: 0,
    });
    assert_eq!(
        fence(&out, 1),
        "```teleprompt scene=vhs include=recordings/tour.tape#2 policy=concurrent"
    );
}

#[test]
fn a_block_dragged_past_its_line_holds() {
    let out = edit(Edit::Hold {
        block: "file-a".into(),
    });
    assert_eq!(
        fence(&out, 1),
        "```teleprompt scene=vhs include=recordings/tour.tape#2"
    );
}

#[test]
fn stretching_multiplies_and_a_stretch_of_one_is_removed() {
    let once = edit(Edit::Stretch {
        block: "look-a".into(),
        by: 1.5,
    });
    assert!(
        fence(&once, 0).ends_with("stretch=1.5"),
        "{}",
        fence(&once, 0)
    );
    let twice = apply(
        &once,
        &Edit::Stretch {
            block: "look-a".into(),
            by: 2.0,
        },
    )
    .unwrap();
    assert!(
        fence(&twice, 0).ends_with("stretch=3"),
        "{}",
        fence(&twice, 0)
    );
    let back = apply(
        &once,
        &Edit::Stretch {
            block: "look-a".into(),
            by: 1.0 / 1.5,
        },
    )
    .unwrap();
    assert_eq!(back, SCRIPT);
}

#[test]
fn a_block_moved_to_another_line_goes_after_it() {
    let out = edit(Edit::Move {
        block: "look-a".into(),
        after: "file".into(),
        word: None,
    });
    let file = out.find("And now the file.").unwrap();
    let moved = out.find("tour.tape#1").unwrap();
    assert!(moved > file, "{out}");
    assert!(!out.contains("\n\n\n"), "no gaps left behind:\n{out}");
    assert!(out.ends_with("```\n") && !out.ends_with("\n\n"), "{out}");
    // It pairs with that line now, and the reparsed script says so.
    let again = apply(
        &out,
        &Edit::Cue {
            block: "file-a".into(),
            word: 2,
        },
    )
    .unwrap();
    assert!(
        again.contains("tour.tape#1 policy=concurrent cue=\"the file\""),
        "{again}"
    );
}

#[test]
fn a_block_moved_up_and_cued_lands_on_the_word() {
    let out = edit(Edit::Move {
        block: "file-a".into(),
        after: "look".into(),
        word: Some(2),
    });
    let look = out.find("{#look}").unwrap();
    let moved = out.find("tour.tape#2").unwrap();
    let file = out.find("And now the file.").unwrap();
    assert!(look < moved && moved < file, "{out}");
    assert!(!out.contains("\n\n\n") && !out.ends_with("\n\n"), "{out}");
    assert!(
        out.contains("tour.tape#2 policy=concurrent cue=\"what is\""),
        "{out}"
    );
}

#[test]
fn unknown_ids_are_named() {
    let e = apply(
        SCRIPT,
        &Edit::Hold {
            block: "nope".into(),
        },
    )
    .unwrap_err();
    assert!(e.contains("`nope`"), "{e}");
}

/// A cue does not run across the end of a sentence: the word that ends
/// one names itself.
#[test]
fn a_cue_at_a_sentence_s_end_is_that_word() {
    let src =
        "# T\n\nWelcome to Acme. Let me show you. {#w}\n\n```teleprompt scene=mock\nwait 1s\n```\n";
    let out = apply(
        src,
        &Edit::Cue {
            block: "w-a".into(),
            word: 2,
        },
    )
    .unwrap();
    assert!(
        out.contains("cue=Acme\n") || out.contains("cue=Acme "),
        "{out}"
    );
}

/// A cue names one place: a phrase the line says twice grows until it
/// is said once.
#[test]
fn a_cue_said_twice_grows_until_it_is_said_once() {
    let src = "# T\n\nList the files, then list the files again. {#w}\n\n```teleprompt scene=mock\nwait 1s\n```\n";
    let out = apply(
        src,
        &Edit::Cue {
            block: "w-a".into(),
            word: 5,
        },
    )
    .unwrap();
    assert!(out.contains("cue=\"the files again\""), "{out}");
}

/// `align` is for concurrent blocks; one that holds loses it, or the
/// script would not compile.
#[test]
fn a_held_block_loses_its_align() {
    let src = "# T\n\nOne line. {#w}\n\n```teleprompt scene=mock policy=concurrent align=end\nwait 1s\n```\n";
    let out = apply(
        src,
        &Edit::Hold {
            block: "w-a".into(),
        },
    )
    .unwrap();
    assert!(out.contains("```teleprompt scene=mock\n"), "{out}");
}

/// Moving a block onto the line it already follows is a cue there.
#[test]
fn a_block_moved_onto_its_own_line_is_cued_in_place() {
    let moved = edit(Edit::Move {
        block: "look-a".into(),
        after: "look".into(),
        word: Some(2),
    });
    let cued = edit(Edit::Cue {
        block: "look-a".into(),
        word: 2,
    });
    assert_eq!(moved, cued);
}
