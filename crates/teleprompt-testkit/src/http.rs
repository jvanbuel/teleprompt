//! An HTTP server for a test to stand in for a speech service: a fixed
//! reply per route, and every request kept to look at afterwards.
//!
//! ```ignore
//! let stub = http::spawn(Reply::ok(pcm)).await;            // every route
//! let stub = http::spawn(BTreeMap::from([("GET /voices", Reply::json(v))])).await;
//! ```

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// What a route answers.
#[derive(Clone)]
pub enum Reply {
    /// This status, content type and body.
    Send(u16, &'static str, Vec<u8>),
    /// 200, claiming a longer body than is sent, then closing: a response
    /// cut off in transit.
    Truncated { body: Vec<u8>, claim: usize },
    /// The connection held open and never answered: a server that hangs.
    Hang,
}

impl Reply {
    pub fn ok(body: impl Into<Vec<u8>>) -> Self {
        Reply::Send(200, "application/octet-stream", body.into())
    }

    pub fn json(v: serde_json::Value) -> Self {
        Reply::Send(200, "application/json", v.to_string().into_bytes())
    }

    pub fn wav(bytes: Vec<u8>) -> Self {
        Reply::Send(200, "audio/wav", bytes)
    }

    /// An error status with this body.
    pub fn error(code: u16, body: impl ToString) -> Self {
        Reply::Send(code, "application/json", body.to_string().into_bytes())
    }
}

/// What the server answers: one reply for every route, or a reply per
/// route ("METHOD /path"), with a 404 for any other.
pub enum Routes {
    Any(Reply),
    Each(BTreeMap<String, Reply>),
}

impl From<Reply> for Routes {
    fn from(reply: Reply) -> Self {
        Routes::Any(reply)
    }
}

impl From<BTreeMap<&'static str, Reply>> for Routes {
    fn from(routes: BTreeMap<&'static str, Reply>) -> Self {
        Routes::Each(
            routes
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        )
    }
}

impl Routes {
    fn reply(&self, route: &str) -> Reply {
        match self {
            Routes::Any(reply) => reply.clone(),
            Routes::Each(routes) => routes
                .get(route)
                .cloned()
                .unwrap_or_else(|| Reply::Send(404, "text/plain", b"not found".to_vec())),
        }
    }
}

/// A request the server was sent.
#[derive(Debug, Clone)]
pub struct Request {
    /// "METHOD /path".
    pub route: String,
    /// The request line and headers.
    pub head: String,
    pub body: Vec<u8>,
}

impl Request {
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("{} sent no JSON ({e}): {}", self.route, self.text()))
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

pub struct Stub {
    /// Where the server is: `http://127.0.0.1:<port>`.
    pub base_url: String,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl Stub {
    /// Every request so far, in the order they came.
    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }

    /// The first request to `route`.
    pub fn request_to(&self, route: &str) -> Request {
        self.requests()
            .into_iter()
            .find(|r| r.route == route)
            .unwrap_or_else(|| panic!("no request to {route}"))
    }

    /// Every request to `route`.
    pub fn requests_to(&self, route: &str) -> Vec<Request> {
        self.requests()
            .into_iter()
            .filter(|r| r.route == route)
            .collect()
    }

    /// The JSON body of the first request to `route`.
    pub fn json_to(&self, route: &str) -> serde_json::Value {
        self.request_to(route).json()
    }

    /// The head of the first request to `route`, lowercased.
    pub fn head_of(&self, route: &str) -> String {
        self.request_to(route).head.to_lowercase()
    }

    /// The first request, whatever its route.
    pub fn first(&self) -> Request {
        self.requests().into_iter().next().expect("no request yet")
    }
}

/// A server on an ephemeral port, answering `routes` until the test ends.
pub async fn spawn(routes: impl Into<Routes>) -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let requests: Arc<Mutex<Vec<Request>>> = Arc::default();
    let (seen, routes) = (requests.clone(), Arc::new(routes.into()));
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let (seen, routes) = (seen.clone(), Arc::clone(&routes));
            tokio::spawn(async move {
                let Some(request) = read_request(&mut socket).await else {
                    return;
                };
                let reply = routes.reply(&request.route);
                seen.lock().unwrap().push(request);
                answer(&mut socket, reply).await;
            });
        }
    });
    Stub { base_url, requests }
}

/// A whole request: its head, and as much body as its Content-Length says.
async fn read_request(socket: &mut TcpStream) -> Option<Request> {
    let mut raw = Vec::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = socket.read(&mut buf).await.unwrap_or(0);
        raw.extend_from_slice(&buf[..n]);
        // Counted in bytes: a body need not be text.
        if let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&raw[..end]).into_owned();
            let length = head
                .lines()
                .find_map(|l| {
                    let l = l.to_lowercase();
                    l.strip_prefix("content-length:")?
                        .trim()
                        .parse::<usize>()
                        .ok()
                })
                .unwrap_or(0);
            if raw.len() >= end + 4 + length || n == 0 {
                let route = head.split(' ').take(2).collect::<Vec<_>>().join(" ");
                let body = raw[end + 4..].to_vec();
                return Some(Request { route, head, body });
            }
        } else if n == 0 {
            return None;
        }
    }
}

async fn answer(socket: &mut TcpStream, reply: Reply) {
    let (head, body) = match reply {
        Reply::Hang => {
            // Until the client gives up and closes its side.
            let _ = socket.read(&mut [0u8; 1]).await;
            return;
        }
        Reply::Send(code, kind, body) => (
            format!(
                "HTTP/1.1 {code} X\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n",
                body.len()
            ),
            body,
        ),
        Reply::Truncated { body, claim } => (
            format!("HTTP/1.1 200 OK\r\nContent-Length: {claim}\r\nConnection: close\r\n\r\n"),
            body,
        ),
    };
    let _ = socket.write_all(head.as_bytes()).await;
    let _ = socket.write_all(&body).await;
    let _ = socket.shutdown().await;
}
