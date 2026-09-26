//! The API of a running server: its script, its clips and the session
//! socket.

use std::io::ErrorKind;
use std::net::TcpStream;
use std::sync::mpsc;
use std::time::Duration;

use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use crate::api::{encode_samples, ClientMessage, Script, ServerMessage, VERSION};

#[derive(Debug, Clone)]
pub struct SessionClient {
    /// Such as `http://127.0.0.1:41234`.
    pub origin: String,
}

/// What goes to the server on the socket.
#[derive(Debug)]
pub enum Outgoing {
    Command(ClientMessage),
    /// Mono samples at the take's rate.
    Audio(Vec<f32>),
    Close,
}

/// What comes back.
#[derive(Debug, PartialEq, Eq)]
pub enum Incoming {
    Message(ServerMessage),
    /// The socket ended; why, if it failed.
    Closed(Option<String>),
}

impl SessionClient {
    pub fn new(origin: impl Into<String>) -> Self {
        Self {
            origin: origin.into(),
        }
    }

    /// A path the server gave, such as a shot's clip, as a URL.
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.origin)
    }

    pub fn script(&self) -> Result<Script, String> {
        let url = self.url(&format!("{VERSION}/script"));
        let response = ureq::get(&url).call().map_err(|e| e.to_string())?;
        serde_json::from_reader(response.into_reader()).map_err(|e| e.to_string())
    }

    /// The bytes at `path`, such as a clip.
    pub fn fetch(&self, path: &str) -> Result<Vec<u8>, String> {
        let response = ureq::get(&self.url(path))
            .call()
            .map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut response.into_reader(), &mut bytes)
            .map_err(|e| e.to_string())?;
        Ok(bytes)
    }

    /// Opens the session socket on its own thread: what is sent on the
    /// returned sender goes to the server, and `on_incoming` hears what
    /// comes back, from that thread.
    pub fn open(
        &self,
        on_incoming: impl Fn(Incoming) + Send + 'static,
    ) -> Result<mpsc::Sender<Outgoing>, String> {
        let host = self
            .origin
            .strip_prefix("http://")
            .ok_or_else(|| format!("not an http origin: {}", self.origin))?
            .to_string();
        let stream = TcpStream::connect(&host).map_err(|e| e.to_string())?;
        let url = format!("ws://{host}{VERSION}/session");
        let (mut socket, _) = tungstenite::client(url.as_str(), MaybeTlsStream::Plain(stream))
            .map_err(|e| match e {
                tungstenite::HandshakeError::Failure(tungstenite::Error::Http(r))
                    if r.status() == 409 =>
                {
                    "another prompter already has the session".to_string()
                }
                e => format!("no session: {e}"),
            })?;
        if let MaybeTlsStream::Plain(s) = socket.get_ref() {
            // Short reads, so what is waiting to be sent is not held up.
            s.set_read_timeout(Some(Duration::from_millis(10)))
                .map_err(|e| e.to_string())?;
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let why = pump(&mut socket, &rx, &on_incoming);
            on_incoming(Incoming::Closed(why));
        });
        Ok(tx)
    }
}

/// Sends what is queued and reads what arrives until either side closes;
/// why it ended, if not asked to.
fn pump(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    outgoing: &mpsc::Receiver<Outgoing>,
    on_incoming: &impl Fn(Incoming),
) -> Option<String> {
    loop {
        loop {
            let message = match outgoing.try_recv() {
                Ok(Outgoing::Command(c)) => Message::text(c.json()),
                Ok(Outgoing::Audio(samples)) => Message::binary(encode_samples(&samples)),
                Ok(Outgoing::Close) | Err(mpsc::TryRecvError::Disconnected) => {
                    let _ = socket.close(None);
                    let _ = socket.flush();
                    return None;
                }
                Err(mpsc::TryRecvError::Empty) => break,
            };
            if let Err(e) = socket.send(message) {
                return Some(format!("lost the server: {e}"));
            }
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                if let Ok(message) = ServerMessage::parse(text.as_str()) {
                    on_incoming(Incoming::Message(message));
                }
            }
            Ok(Message::Close(_)) => return Some("the server closed the session".into()),
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                return Some("the server closed the session".into())
            }
            Err(e) => return Some(format!("lost the server: {e}")),
        }
    }
}
