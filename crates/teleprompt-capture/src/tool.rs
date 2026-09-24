//! Running the external programs a capture backend drives.

use std::process::{Command, Output, Stdio};

/// Whether `program` can be started at all. Its exit status is not
/// consulted: a tool that runs and fails is present, and its failure is
/// reported where it happens.
pub fn installed(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// "`a` and `b` is not on PATH" for the programs that cannot be started,
/// or `None` when every one can. What `CaptureBackend::unavailable` says.
pub fn missing(programs: &[&str]) -> Option<String> {
    let absent: Vec<&str> = programs.iter().copied().filter(|p| !installed(p)).collect();
    (!absent.is_empty()).then(|| format!("{} is not on PATH", absent.join(" and ")))
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
    fn missing_names_every_absent_program() {
        assert_eq!(
            missing(&["teleprompt-no-such-a", "teleprompt-no-such-b"]).as_deref(),
            Some("teleprompt-no-such-a and teleprompt-no-such-b is not on PATH")
        );
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
