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

/// How [`missing`] ends its reason for one program; " are not on PATH" is
/// for several. A caller can tell a backend held back by tools from one held
/// back by something else with [`programs_in`].
pub const NOT_ON_PATH: &str = " is not on PATH";
const ARE_NOT_ON_PATH: &str = " are not on PATH";

/// "`a` is not on PATH", "`a` and `b` are not on PATH" or "`a`, `b` and `c`
/// are not on PATH" for the programs that cannot be started, or `None` when
/// every one can. What `CaptureBackend::unavailable` says.
pub fn missing(programs: &[&str]) -> Option<String> {
    let absent: Vec<&str> = programs.iter().copied().filter(|p| !installed(p)).collect();
    let (last, rest) = absent.split_last()?;
    Some(if rest.is_empty() {
        format!("{last}{NOT_ON_PATH}")
    } else {
        format!("{} and {last}{ARE_NOT_ON_PATH}", rest.join(", "))
    })
}

/// The programs a reason made by [`missing`] names, or `None` when it is a
/// reason of any other kind.
pub fn programs_in(reason: &str) -> Option<Vec<&str>> {
    let names = reason
        .strip_suffix(NOT_ON_PATH)
        .or_else(|| reason.strip_suffix(ARE_NOT_ON_PATH))?;
    Some(
        names
            .split([','])
            .flat_map(|part| part.split(" and "))
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .collect(),
    )
}

/// [`missing`] for the programs among `tools`: what a backend that runs
/// exactly what it needs cannot run.
pub fn missing_of(tools: &[&Tool]) -> Option<String> {
    let programs: Vec<&str> = tools
        .iter()
        .filter_map(|t| match t.found {
            Found::Program(p) => Some(p),
            _ => None,
        })
        .collect();
    missing(&programs)
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

    #[test]
    fn missing_names_every_absent_program_in_good_english() {
        let a = "teleprompt-no-such-a";
        let (b, c) = ("teleprompt-no-such-b", "teleprompt-no-such-c");
        assert_eq!(
            missing(&[a]).as_deref(),
            Some("teleprompt-no-such-a is not on PATH")
        );
        assert_eq!(
            missing(&[a, b]).as_deref(),
            Some("teleprompt-no-such-a and teleprompt-no-such-b are not on PATH")
        );
        assert_eq!(
            missing(&[a, b, c]).as_deref(),
            Some(
                "teleprompt-no-such-a, teleprompt-no-such-b and teleprompt-no-such-c \
                 are not on PATH"
            )
        );
        assert_eq!(missing(&[]), None);
    }

    #[test]
    fn the_programs_in_a_reason_are_the_ones_it_names() {
        for programs in [vec!["a"], vec!["a", "b"], vec!["a", "b", "c"]] {
            let reason = match programs.as_slice() {
                [only] => format!("{only}{NOT_ON_PATH}"),
                [rest @ .., last] => format!("{} and {last}{ARE_NOT_ON_PATH}", rest.join(", ")),
                [] => unreachable!(),
            };
            assert_eq!(programs_in(&reason), Some(programs));
        }
        assert_eq!(programs_in("macOS only"), None);
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
