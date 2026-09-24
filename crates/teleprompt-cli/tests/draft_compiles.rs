//! The second seam `from` was agreed at: a drafted script is a script.
//!
//! The unit tests in `teleprompt-core` check what the draft *says*. This
//! checks that what it says survives the command an author runs next — with
//! the adapters this build actually ships, which is why it lives here and
//! not beside the drafting code.

use std::path::PathBuf;

use teleprompt_cli::cmd::check::run_check;
use teleprompt_cli::draft::draft;
use teleprompt_cli::project::Project;

const README: &str = "\
# Acme

Acme builds your project and deploys it. This paragraph is the one the
narrator reads first.

```bash
npm install acme
acme build
```

Once it is installed, deploying is a single command.

```bash
acme deploy --prod
```

The configuration file looks like this.

```json
{\"target\": \"production\"}
```
";

/// Writes `source` into a scaffolded project and runs `check` over it.
fn check(source: &str) -> Result<Vec<String>, Vec<String>> {
    let dir = std::env::temp_dir().join(format!(
        "tp-draft-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();

    let script: PathBuf = dir.join("scripts/drafted.md");
    std::fs::write(&script, source).unwrap();
    run_check(&Project::discover(&dir).unwrap(), &script, "en")
}

#[test]
fn a_drafted_script_passes_check() {
    let warnings = check(&draft(README, "Acme")).unwrap_or_else(|e| panic!("{e:#?}"));
    // Two generated tapes, neither read by a human yet.
    let unreviewed: Vec<&String> = warnings.iter().filter(|w| w.contains("review")).collect();
    assert_eq!(unreviewed.len(), 2, "{warnings:#?}");

    // The warning is read by a person on every `check` until they act on it,
    // so it names the block and reads as one sentence.
    for w in &unreviewed {
        assert!(w.contains("`review=pending`"), "{w}");
        assert!(
            !w.contains("  "),
            "a wrapped literal leaked its indentation: {w:?}"
        );
    }
}

#[test]
fn a_command_with_a_quote_in_it_survives_into_the_tape() {
    // The tape is generated, so nobody proof-read it: an unescaped quote
    // would close `Type`'s string early, and the adapter is where that
    // surfaces.
    let script = draft(
        "Deploy it.\n\n```bash\nacme deploy --message \"ship it\"\n```\n",
        "Acme",
    );
    check(&script).unwrap_or_else(|e| panic!("{e:#?}"));
}

#[test]
fn a_document_that_opens_with_prose_still_compiles() {
    // Every line and action block must belong to a chapter, and a README that
    // opens with a sentence has none.
    let script = draft("Acme deploys your project.\n", "Acme");
    check(&script).unwrap_or_else(|e| panic!("{e:#?}"));
}
