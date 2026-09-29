//! An HTTP server for a test to stand in for a voice service: fixed
//! replies by route, and every request's route and body kept.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub type Seen = Arc<Mutex<Vec<(String, String)>>>;

/// A route's reply: its body and content type.
pub type Routes = BTreeMap<String, (Vec<u8>, &'static str)>;

/// Answers `routes`, keyed by "METHOD /path"; records each request.
pub async fn serve(routes: Routes) -> (String, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen: Seen = Arc::default();
    let log = seen.clone();
    let routes = Arc::new(routes);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let (log, routes) = (log.clone(), routes.clone());
            tokio::spawn(async move {
                let mut raw = Vec::new();
                let mut buf = [0u8; 65536];
                loop {
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    raw.extend_from_slice(&buf[..n]);
                    // Counted in bytes: a WAV body is not text.
                    let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") else {
                        if n == 0 {
                            return;
                        }
                        continue;
                    };
                    let head = String::from_utf8_lossy(&raw[..end]).to_string();
                    let length = head
                        .lines()
                        .find_map(|l| {
                            l.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if raw.len() < end + 4 + length && n > 0 {
                        continue;
                    }
                    let body = String::from_utf8_lossy(&raw[end + 4..]).to_string();
                    let route: String = head.split(' ').take(2).collect::<Vec<_>>().join(" ");
                    log.lock().unwrap().push((route.clone(), body));
                    let (reply, kind) = routes
                        .get(&route)
                        .cloned()
                        .unwrap_or((b"no".to_vec(), "text/plain"));
                    let head = format!("HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", reply.len());
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(&reply).await;
                    return;
                }
            });
        }
    });
    (url, seen)
}
