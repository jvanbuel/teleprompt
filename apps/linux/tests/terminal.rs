use std::path::Path;

use teleprompt_gtk::terminal::command;

fn argv() -> Vec<String> {
    vec!["teleprompt".into(), "record".into(), "s.md".into()]
}

fn run(
    configured: Option<&str>,
    env: Option<&str>,
    installed: &[&str],
) -> Result<Vec<String>, String> {
    command(
        configured,
        env,
        |p| installed.contains(&p),
        Path::new("/p"),
        &argv(),
    )
}

const TAIL: &[&str] = &[
    "sh",
    "-c",
    "cd \"$0\" && exec \"$@\"",
    "/p",
    "teleprompt",
    "record",
    "s.md",
];

fn with(head: &[&str]) -> Vec<String> {
    head.iter().chain(TAIL).map(|s| s.to_string()).collect()
}

#[test]
fn the_terminal_set_in_settings_comes_first() {
    let got = run(
        Some("ghostty -e"),
        Some("xterm"),
        &["xdg-terminal-exec", "kitty"],
    )
    .unwrap();
    assert_eq!(got, with(&["ghostty", "-e"]));
}

#[test]
fn then_the_desktops_choice_then_terminal_env() {
    assert_eq!(
        run(None, Some("xterm"), &["xdg-terminal-exec"]).unwrap(),
        with(&["xdg-terminal-exec"])
    );
    assert_eq!(
        run(None, Some("foot"), &["kitty"]).unwrap(),
        with(&["foot", "-e"])
    );
    assert_eq!(run(Some("  "), None, &["kitty"]).unwrap(), with(&["kitty"]));
}

#[test]
fn a_known_terminal_is_given_the_command_its_own_way() {
    assert_eq!(
        run(None, None, &["gnome-terminal", "xterm"]).unwrap(),
        with(&["gnome-terminal", "--"])
    );
    assert_eq!(
        run(None, None, &["wezterm"]).unwrap(),
        with(&["wezterm", "start", "--"])
    );
}

#[test]
fn no_terminal_says_how_to_set_one() {
    let err = run(None, None, &[]).unwrap_err();
    assert!(err.contains("Settings"), "{err}");
}
