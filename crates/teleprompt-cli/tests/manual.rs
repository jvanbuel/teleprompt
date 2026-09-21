//! The manual teleprompt writes about itself.
//!
//! `manual/scripts/cli.md` narrates the command-line interface, and its
//! action blocks are VHS tapes running the commands it describes. Compiling
//! it here is what keeps the claim honest: a manual that lived only in the
//! repository would rot quietly, and one that only a human ever compiled
//! would rot loudly and late.
//!
//! The committed timeline is compiled from a *cold* cache on purpose. Every
//! narration duration in it is a word-count estimate, which is reproducible
//! on any machine with nothing installed; a timeline dubbed from a warm
//! cache would be measured, and CI — which starts with an empty
//! `.teleprompt/cache` every run — could never reproduce it.
//!
//! Which is why these tests copy the manual somewhere else before compiling
//! it. A contributor who has run `teleprompt dub manual/scripts/cli.md`
//! locally has a warm `manual/.teleprompt/cache`, and every narration in it
//! legitimately becomes `measured` — same numbers under the null backend,
//! different provenance, and `diff` says so. Compiling in place would fail
//! this suite on exactly the machines that had exercised the manual most.

use std::path::{Path, PathBuf};

use teleprompt_cli::cmd::{check::run_check, diff::run_diff, plan::run_plan};
use teleprompt_cli::project::Project;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../")
}

/// The manual, in a scratch directory with a guaranteed-cold cache.
fn manual() -> (Project, PathBuf) {
    let src = repo().join("manual");
    let dir = std::env::temp_dir().join(format!(
        "teleprompt-manual-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    for rel in [
        Path::new("teleprompt.toml"),
        Path::new("scripts/cli.md"),
        Path::new("timelines/cli.en.json"),
    ] {
        let to = dir.join(rel);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::copy(src.join(rel), &to)
            .unwrap_or_else(|e| panic!("copying {}: {e}", rel.display()));
    }
    let script = dir.join("scripts/cli.md");
    (
        Project::discover(&dir).expect("the manual is a teleprompt project"),
        script,
    )
}

#[test]
fn the_manual_compiles() {
    let (p, s) = manual();
    let warnings = run_check(&p, &s, "en").expect("the manual must be a valid script");
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
}

#[test]
fn the_committed_timeline_is_current() {
    let (p, s) = manual();
    let d = run_diff(&p, &s, "en").unwrap();
    assert!(
        d.is_empty(),
        "the manual has drifted from `manual/timelines/cli.en.json`; re-run\n  \
         cargo run -- plan manual/scripts/cli.md --format json > manual/timelines/cli.en.json\n\
         {}",
        d.render()
    );
}

#[test]
fn every_terminal_action_is_timed_exactly() {
    // The claim the tape adapter makes and the reason `plan` can report a
    // terminal scene's pacing with no terminal anywhere: a tape states its
    // own timing, so nothing here is a guess.
    let (p, s) = manual();
    let out = run_plan(&p, &s, "en").unwrap();
    let mut tapes = 0;
    for e in &out.timeline.entries {
        let Some(a) = &e.action else { continue };
        if a.adapter != "vhs" {
            continue;
        }
        tapes += 1;
        assert_eq!(
            a.duration_source, "exact",
            "span {} is {} rather than exact",
            a.span, a.duration_source
        );
    }
    assert!(tapes > 0, "the manual must exercise the tape adapter");
}

#[test]
fn the_manual_demonstrates_every_pacing_policy() {
    // A manual that only ever used the default policy would document the
    // tool it is not.
    let (p, s) = manual();
    let out = run_plan(&p, &s, "en").unwrap();
    let policies: std::collections::BTreeSet<&str> = out
        .timeline
        .entries
        .iter()
        .map(|e| e.policy.as_str())
        .collect();
    for expected in ["hold", "concurrent", "stretch-action", "trim-action"] {
        assert!(policies.contains(expected), "missing policy {expected}");
    }
}
