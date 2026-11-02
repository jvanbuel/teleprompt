//! The locale a script is written in, which a command compiles it for when
//! given none.

use teleprompt_project::project::Project;
use teleprompt_project::registry::Registry;

/// Front matter's `locales.source` wins over the project's, and a script
/// whose front matter says nothing is in the project's locale.
#[test]
fn a_scripts_front_matter_names_its_source_locale() {
    let dir = teleprompt_testkit::test_dir("source-locale");
    std::fs::write(
        dir.path().join("teleprompt.toml"),
        "[locales]\nsource = \"fr\"\n",
    )
    .unwrap();
    let dutch = dir.path().join("dutch.md");
    std::fs::write(&dutch, "---\nlocales: { source: nl }\n---\n\n# Intro\n").unwrap();
    let plain = dir.path().join("plain.md");
    std::fs::write(&plain, "# Intro\n").unwrap();
    let project = Project::discover(dir.path(), Registry::mock()).unwrap();

    assert_eq!(project.source_locale(&dutch), "nl");
    assert_eq!(project.source_locale(&plain), "fr");
    assert_eq!(
        project.source_locale_in("---\nlocales: { source: de }\n---\n\n# Intro\n"),
        "de"
    );
}
