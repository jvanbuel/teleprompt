//! The author's own terminal: how to open a window of it running a command.
//! Session mode records there, not in a terminal of the app's.

use std::path::Path;

/// Terminals as they take a command to run, in the order they are looked
/// for when nothing says which: the flag before the command, if any.
const KNOWN: &[(&str, &[&str])] = &[
    ("ghostty", &["-e"]),
    ("kitty", &[]),
    ("alacritty", &["-e"]),
    ("foot", &[]),
    ("wezterm", &["start", "--"]),
    ("konsole", &["-e"]),
    ("gnome-terminal", &["--"]),
    ("xfce4-terminal", &["-x"]),
    ("x-terminal-emulator", &["-e"]),
    ("xterm", &["-e"]),
];

/// The command line opening the author's terminal on `argv`, run in `dir`:
/// the terminal `configured` in Settings (its command and any flag, as
/// `kitty` or `ghostty -e`), else `xdg-terminal-exec`, else `$TERMINAL`,
/// else the first of [`KNOWN`] that `installed` finds.
pub fn command(
    configured: Option<&str>,
    terminal_env: Option<&str>,
    installed: impl Fn(&str) -> bool,
    dir: &Path,
    argv: &[String],
) -> Result<Vec<String>, String> {
    let mut line: Vec<String> = if let Some(c) = configured.filter(|c| !c.trim().is_empty()) {
        c.split_whitespace().map(str::to_string).collect()
    } else if installed("xdg-terminal-exec") {
        vec!["xdg-terminal-exec".into()]
    } else if let Some(t) = terminal_env.filter(|t| !t.is_empty()) {
        vec![t.to_string(), "-e".into()]
    } else {
        let (name, flag) = KNOWN
            .iter()
            .find(|(name, _)| installed(name))
            .ok_or("no terminal found: set yours in Settings, e.g. `kitty` or `ghostty -e`")?;
        std::iter::once(*name)
            .chain(flag.iter().copied())
            .map(str::to_string)
            .collect()
    };
    // Not every terminal opens where it is started from, so the command
    // goes there itself.
    line.extend(["sh", "-c", "cd \"$0\" && exec \"$@\""].map(str::to_string));
    line.push(dir.display().to_string());
    line.extend(argv.iter().cloned());
    Ok(line)
}

/// Whether `program` is on the PATH.
pub fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .any(|dir| dir.join(program).is_file())
}
