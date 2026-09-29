//! A translator of the author's: a shell command given the request as JSON
//! on stdin, answering on stdout.

use std::process::Stdio;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::{Request, Response};

/// The program and how long it may take over one batch.
#[derive(Debug, Clone)]
pub struct Program {
    pub run: String,
    pub timeout_ms: u64,
}

impl Program {
    pub fn new(run: &str) -> Self {
        Program {
            run: run.to_string(),
            timeout_ms: crate::TIMEOUT_MS,
        }
    }
}

pub(crate) async fn translate(program: &Program, request: &Request) -> Result<Response, String> {
    let command = &program.run;
    let mut child = Command::new("sh")
        .args(["-c", command])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("cannot run `{command}`: {e}"))?;
    let input = serde_json::to_vec(request).map_err(|e| e.to_string())?;
    let mut stdin = child.stdin.take();
    // Written while the answer is read, so a program that answers as it
    // reads cannot fill its output pipe and wait on us for ever. One that
    // answers without reading its input closes the pipe early; its answer
    // still counts.
    let write = async {
        if let Some(stdin) = stdin.as_mut() {
            let _ = stdin.write_all(&input).await;
        }
        drop(stdin.take());
    };
    let run = async { tokio::join!(write, child.wait_with_output()).1 };
    let out = tokio::time::timeout(std::time::Duration::from_millis(program.timeout_ms), run)
        .await
        .map_err(|_| crate::unanswered(&format!("`{command}`"), program.timeout_ms))?
        .map_err(|e| format!("`{command}` failed: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "`{command}` exited with {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| {
        format!("`{command}` did not answer with {{\"items\": [{{\"id\", \"text\"}}]}}: {e}")
    })
}
