// Shared across every integration test target under `tests/` (`synth.rs`,
// `voices.rs`, …). Each target exercises only the subset of this API its
// own scenarios need — `voices.rs` never triggers `Reply::Hang`, for
// instance — so from any single target's point of view the rest of this
// module looks unused. That is a property of how cargo compiles each
// integration test as its own crate, not of this module actually having
// dead code.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// What the stub should do for the next request.
#[derive(Clone)]
pub enum Reply {
    /// 200 with these bytes as the body.
    Ok(Vec<u8>),
    /// 200, but claim a longer body than is sent, then close. Exercises the
    /// truncated-response path.
    Truncated { body: Vec<u8>, claim: usize },
    /// A non-200 with this body.
    Status(u16, String),
    /// A non-200 carrying one extra response header. Exercises the paths
    /// that read a header off a failure — `Retry-After`, so far.
    StatusWithHeader {
        code: u16,
        body: String,
        header: (&'static str, &'static str),
    },
    /// Accept the connection and never answer. Exercises the timeout path.
    Hang,
}

pub struct Stub {
    pub base_url: String,
    pub requests: Arc<Mutex<Vec<String>>>,
}

/// Spawns a one-route HTTP/1.1 server on an ephemeral port. Handles requests
/// until dropped. Returns the URL to point `base_url` at.
pub async fn spawn(reply: Reply) -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let reply = reply.clone();
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 65536];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let raw = String::from_utf8_lossy(&buf[..n]).to_string();
                seen.lock().unwrap().push(raw);

                match reply {
                    Reply::Hang => {
                        // Hold the connection open with no response.
                        hang_until_peer_gives_up(&mut socket).await;
                    }
                    Reply::Ok(body) => {
                        let head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: \
                             application/octet-stream\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = socket.write_all(head.as_bytes()).await;
                        let _ = socket.write_all(&body).await;
                    }
                    Reply::Truncated { body, claim } => {
                        let head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {claim}\r\nConnection: \
                             close\r\n\r\n"
                        );
                        let _ = socket.write_all(head.as_bytes()).await;
                        let _ = socket.write_all(&body).await;
                        // Close without sending the rest.
                    }
                    Reply::StatusWithHeader {
                        code,
                        body,
                        header: (name, value),
                    } => {
                        let head = format!(
                            "HTTP/1.1 {code} X\r\n{name}: {value}\r\nContent-Length: \
                             {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = socket.write_all(head.as_bytes()).await;
                        let _ = socket.write_all(body.as_bytes()).await;
                    }
                    Reply::Status(code, body) => {
                        let head = format!(
                            "HTTP/1.1 {code} X\r\nContent-Length: {}\r\nConnection: \
                             close\r\n\r\n",
                            body.len()
                        );
                        let _ = socket.write_all(head.as_bytes()).await;
                        let _ = socket.write_all(body.as_bytes()).await;
                    }
                }
                let _ = socket.shutdown().await;
            });
        }
    });

    Stub {
        base_url: format!("http://{addr}"),
        requests,
    }
}

/// Holds the connection open with no response, until the client (the one
/// exercising the timeout path) gives up and closes its side.
async fn hang_until_peer_gives_up(socket: &mut tokio::net::TcpStream) {
    let mut sink = [0u8; 1];
    // Reading blocks until the peer gives up; that is the hang we want.
    let _ = socket.read(&mut sink).await;
}

impl Stub {
    /// The JSON body of the first request, parsed.
    pub fn first_body(&self) -> serde_json::Value {
        let raw = self.requests.lock().unwrap()[0].clone();
        let body = raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("body was not JSON: {e}\n{raw}"))
    }

    pub fn first_request_line(&self) -> String {
        self.requests.lock().unwrap()[0]
            .lines()
            .next()
            .unwrap_or("")
            .to_string()
    }
}
