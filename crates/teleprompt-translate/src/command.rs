//! A translator of the author's: a shell command given the request as JSON
//! on stdin, answering on stdout.

use std::process::Stdio;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::{Request, Response};

pub(crate) async fn translate(command: &str, request: &Request) -> Result<Response, String> {
    let mut child = Command::new("sh")
        .args(["-c", command])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run `{command}`: {e}"))?;
    let input = serde_json::to_vec(request).map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        // A program that answers without reading its input closes the pipe
        // early; its answer still counts.
        let _ = stdin.write_all(&input).await;
    }
    let out = child
        .wait_with_output()
        .await
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
