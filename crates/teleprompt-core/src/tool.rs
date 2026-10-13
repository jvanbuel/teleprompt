//! The programs a plugin needs, as `teleprompt setup` lists and installs
//! them, and running them.

use std::path::Path;
use std::process::{Command, Output, Stdio};

/// Something a plugin needs that teleprompt does not ship: a program, a
/// package in the project, a model, or a server the author runs. A plugin
/// declares each, with its license and how each package manager installs
/// it, and `teleprompt setup` lists it, says its license, and installs it
/// with the author's own manager (docs/design.md#what-teleprompt-ships).
#[derive(Debug)]
pub struct Tool {
    pub name: &'static str,
    /// What teleprompt uses it for.
    pub what: &'static str,
    pub license: &'static str,
    pub home: &'static str,
    /// What to do instead, for what `setup` does not install.
    pub guide: Option<&'static str>,
    pub found: Found,
    /// The command per manager. `{models}` is the models directory.
    pub install: &'static [(Manager, &'static str)],
    /// How much it downloads, where that is known: a model's size.
    pub download_mb: Option<u32>,
}

/// How to tell a tool is here.
#[derive(Debug, Clone, Copy)]
pub enum Found {
    /// A program that runs.
    Program(&'static str),
    /// An npm package in the project's `node_modules`.
    Package(&'static str),
    /// A directory in the models directory.
    Model(&'static str),
    /// Nothing `setup` can check: the author's own project or server.
    Unknowable,
}

/// A way to install software that may be on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manager {
    Brew,
    Apt,
    Dnf,
    Pacman,
    Go,
    Cargo,
    Pipx,
    /// npm, in the project: for tools a project depends on.
    Npm,
    /// `curl` and `tar`, for a download unpacked into the models directory.
    Download,
}

impl Manager {
    pub const ALL: [Manager; 9] = [
        Manager::Brew,
        Manager::Apt,
        Manager::Dnf,
        Manager::Pacman,
        Manager::Go,
        Manager::Cargo,
        Manager::Pipx,
        Manager::Npm,
        Manager::Download,
    ];

    /// Its name in a plugin's description.
    pub fn name(self) -> &'static str {
        match self {
            Manager::Brew => "brew",
            Manager::Apt => "apt",
            Manager::Dnf => "dnf",
            Manager::Pacman => "pacman",
            Manager::Go => "go",
            Manager::Cargo => "cargo",
            Manager::Pipx => "pipx",
            Manager::Npm => "npm",
            Manager::Download => "download",
        }
    }

    pub fn from_name(name: &str) -> Option<Manager> {
        Manager::ALL.into_iter().find(|m| m.name() == name)
    }

    /// The programs it needs on PATH.
    pub fn programs(self) -> &'static [&'static str] {
        match self {
            Manager::Brew => &["brew"],
            Manager::Apt => &["apt-get"],
            Manager::Dnf => &["dnf"],
            Manager::Pacman => &["pacman"],
            Manager::Go => &["go"],
            Manager::Cargo => &["cargo"],
            Manager::Pipx => &["pipx"],
            Manager::Npm => &["npm"],
            Manager::Download => &["curl", "tar"],
        }
    }
}

impl Tool {
    /// The command that installs it with the first of `managers` that can,
    /// or `None` when none of them does.
    pub fn command(
        &self,
        managers: impl IntoIterator<Item = Manager>,
        models: &Path,
    ) -> Option<String> {
        managers.into_iter().find_map(|m| {
            self.install
                .iter()
                .find(|(with, _)| *with == m)
                .map(|(_, c)| c.replace("{models}", &shell_quote(&models.display().to_string())))
        })
    }

    /// Whether it is here, or `None` when `setup` cannot tell.
    pub fn installed(&self, project: &Path, models: &Path) -> Option<bool> {
        match self.found {
            Found::Program(p) => Some(installed(p)),
            Found::Package(p) => Some(project.join("node_modules").join(p).is_dir()),
            Found::Model(m) => Some(models.join(m).is_dir()),
            Found::Unknowable => None,
        }
    }
}

fn shell_quote(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-~".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// What renders the video, and most scene plugins capture with.
pub static FFMPEG: Tool = Tool {
    name: "ffmpeg",
    what: "renders the video, and captures media, slide and terminal scenes",
    license: "LGPL-2.1-or-later, or GPL-2.0-or-later as many builds are",
    home: "https://ffmpeg.org",
    guide: None,
    found: Found::Program("ffmpeg"),
    install: &[
        (Manager::Brew, "brew install ffmpeg"),
        (Manager::Apt, "sudo apt-get install -y ffmpeg"),
        (Manager::Dnf, "sudo dnf install -y ffmpeg-free"),
        (
            Manager::Pacman,
            "sudo pacman -S --needed --noconfirm ffmpeg",
        ),
    ],
    download_mb: None,
};

/// What the JavaScript tools run on.
pub static NODE: Tool = Tool {
    name: "node",
    what: "runs Playwright, Remotion and Slidev",
    license: "MIT",
    home: "https://nodejs.org",
    guide: None,
    found: Found::Program("node"),
    install: &[
        (Manager::Brew, "brew install node"),
        (Manager::Apt, "sudo apt-get install -y nodejs npm"),
        (Manager::Dnf, "sudo dnf install -y nodejs npm"),
        (
            Manager::Pacman,
            "sudo pacman -S --needed --noconfirm nodejs npm",
        ),
    ],
    download_mb: None,
};

/// Whether `program` can be started at all: one that runs and fails is
/// present, its failure reported where it happens.
pub fn installed(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Why a scene plugin or recorder cannot run here.
#[derive(Debug, Clone)]
pub enum Unavailable {
    /// These tools' programs are not on PATH: what `teleprompt setup`
    /// installs.
    Missing(Vec<&'static Tool>),
    /// Anything else, as the backend words it.
    Other(String),
}

impl Unavailable {
    /// The programs it names as missing; none for [`Unavailable::Other`].
    pub fn programs(&self) -> Vec<&'static str> {
        match self {
            Self::Missing(tools) => tools.iter().filter_map(|t| program(t)).collect(),
            Self::Other(_) => Vec::new(),
        }
    }
}

/// "`a` is not on PATH", "`a` and `b` are not on PATH" or "`a`, `b` and
/// `c` are not on PATH"; or the other reason as given.
impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Self::Other(why) = self {
            return f.write_str(why);
        }
        match self.programs().split_last() {
            None => f.write_str("nothing is missing"),
            Some((last, [])) => write!(f, "{last} is not on PATH"),
            Some((last, rest)) => write!(f, "{} and {last} are not on PATH", rest.join(", ")),
        }
    }
}

fn program(tool: &Tool) -> Option<&'static str> {
    match tool.found {
        Found::Program(p) => Some(p),
        _ => None,
    }
}

/// The tools among `tools` whose program cannot be started, or `None`
/// when every one can: what a backend that runs exactly what it needs
/// cannot run.
pub fn missing_of(tools: &[&'static Tool]) -> Option<Unavailable> {
    let absent: Vec<&'static Tool> = tools
        .iter()
        .copied()
        .filter(|t| program(t).is_some_and(|p| !installed(p)))
        .collect();
    (!absent.is_empty()).then_some(Unavailable::Missing(absent))
}

/// The last `keep` non-blank lines of `text`, joined with " / ": the part
/// of a failing program's output that says why.
pub fn tail(text: &str, keep: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let from = lines.len().saturating_sub(keep);
    lines[from..].join(" / ")
}

/// Runs `command` to completion with stdin closed and its output captured.
///
/// A program that cannot be started is "`program` could not be run: …"; one
/// that exits unsuccessfully is "`what` exited `status`: …" with the last
/// `keep` lines of its stderr. Either is the reason a backend reports.
pub fn run(command: &mut Command, what: &str, keep: usize) -> Result<Output, String> {
    let program = command.get_program().to_string_lossy().to_string();
    let ran = command
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{program} could not be run: {e}"))?;
    if ran.status.success() {
        return Ok(ran);
    }
    Err(format!(
        "{what} exited {}: {}",
        ran.status,
        tail(&String::from_utf8_lossy(&ran.stderr), keep)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tool run as `program`, which is on no machine.
    macro_rules! absent {
        ($program:literal) => {{
            static TOOL: Tool = Tool {
                name: $program,
                what: "",
                license: "",
                home: "",
                guide: None,
                found: Found::Program($program),
                install: &[],
                download_mb: None,
            };
            &TOOL
        }};
    }

    /// The words every warning about a missing tool has always used.
    #[test]
    fn a_missing_tool_reads_in_good_english() {
        let a: &'static Tool = absent!("teleprompt-no-such-a");
        let b: &'static Tool = absent!("teleprompt-no-such-b");
        let c: &'static Tool = absent!("teleprompt-no-such-c");
        let said = |tools: &[&'static Tool]| missing_of(tools).map(|u| u.to_string());
        assert_eq!(
            said(&[a]).as_deref(),
            Some("teleprompt-no-such-a is not on PATH")
        );
        assert_eq!(
            said(&[a, b]).as_deref(),
            Some("teleprompt-no-such-a and teleprompt-no-such-b are not on PATH")
        );
        assert_eq!(
            said(&[a, b, c]).as_deref(),
            Some(
                "teleprompt-no-such-a, teleprompt-no-such-b and teleprompt-no-such-c \
                 are not on PATH"
            )
        );
        assert!(missing_of(&[]).is_none());
    }

    #[test]
    fn only_the_tools_not_on_path_are_missing() {
        let here: &'static Tool = absent!("sh");
        let gone: &'static Tool = absent!("teleprompt-no-such-a");
        let missing = missing_of(&[here, gone]).expect("one is missing");
        assert_eq!(missing.programs(), ["teleprompt-no-such-a"]);
    }

    #[test]
    fn another_reason_is_said_as_given_and_names_no_program() {
        let why = Unavailable::Other("this is not a Mac".into());
        assert_eq!(why.to_string(), "this is not a Mac");
        assert!(why.programs().is_empty());
    }

    #[test]
    fn tail_keeps_the_last_non_blank_lines() {
        assert_eq!(tail("one\n\ntwo\n  \nthree\n", 2), "two / three");
        assert_eq!(tail("", 4), "");
    }

    #[test]
    fn run_reports_a_program_that_cannot_start() {
        let why = run(&mut Command::new("teleprompt-no-such-tool"), "it", 4).unwrap_err();
        assert!(
            why.starts_with("teleprompt-no-such-tool could not be run: "),
            "{why}"
        );
    }
}
