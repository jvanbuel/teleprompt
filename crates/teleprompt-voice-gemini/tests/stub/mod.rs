// The Gemini API's routes, as far as teleprompt uses them. Shared by the
// test targets, each of which uses part of it.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A reply: status, content type and body.
#[derive(Clone)]
pub struct Reply(pub u16, pub &'static str, pub Vec<u8>);

impl Reply {
    pub fn json(v: serde_json::Value) -> Self {
        Reply(200, "application/json", v.to_string().into_bytes())
    }
    pub fn status(code: u16, v: serde_json::Value) -> Self {
        Reply(code, "application/json", v.to_string().into_bytes())
    }
}

/// Each request's method and path, its head, then its raw body.
type Seen = Arc<Mutex<Vec<(String, String, Vec<u8>)>>>;

pub struct Stub {
    pub base_url: String,
    pub requests: Seen,
}

/// Serves `routes`, keyed by "METHOD /path"; anything else is a 404.
pub async fn spawn(routes: BTreeMap<&'static str, Reply>) -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    let routes = Arc::new(routes);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let (seen, routes) = (seen.clone(), routes.clone());
            tokio::spawn(async move {
                let raw = read_request(&mut socket).await;
                let split = raw
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .unwrap_or(raw.len());
                let head = String::from_utf8_lossy(&raw[..split]).to_string();
                let body = raw.get(split + 4..).unwrap_or_default().to_vec();
                let line = head.lines().next().unwrap_or("").to_string();
                let route: String = line.split(' ').take(2).collect::<Vec<_>>().join(" ");
                seen.lock()
                    .unwrap()
                    .push((route.clone(), head.clone(), body));
                let Reply(code, kind, body) = routes.get(route.as_str()).cloned().unwrap_or(Reply(
                    404,
                    "text/plain",
                    b"not found".to_vec(),
                ));
                let head = format!(
                    "HTTP/1.1 {code} X\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(&body).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    Stub {
        base_url: format!("http://{addr}"),
        requests,
    }
}

/// A whole request: its head, and as much body as its Content-Length says.
async fn read_request(socket: &mut tokio::net::TcpStream) -> Vec<u8> {
    let mut raw = Vec::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = socket.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            return raw;
        }
        raw.extend_from_slice(&buf[..n]);
        let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") else {
            continue;
        };
        let head = String::from_utf8_lossy(&raw[..end]).to_lowercase();
        let length = head
            .lines()
            .find_map(|l| l.strip_prefix("content-length:"))
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(0);
        if raw.len() >= end + 4 + length {
            return raw;
        }
    }
}

impl Stub {
    /// The body of the request to `route`, as JSON.
    pub fn json_to(&self, route: &str) -> serde_json::Value {
        let requests = self.requests.lock().unwrap();
        let (_, _, body) = requests
            .iter()
            .find(|(r, _, _)| r == route)
            .unwrap_or_else(|| panic!("no request to {route}"));
        serde_json::from_slice(body).unwrap()
    }

    /// The head of the request to `route`, lowercased.
    pub fn head_of(&self, route: &str) -> String {
        let requests = self.requests.lock().unwrap();
        let (_, head, _) = requests
            .iter()
            .find(|(r, _, _)| r == route)
            .unwrap_or_else(|| panic!("no request to {route}"));
        head.to_lowercase()
    }
}
