//! Driving a terminal from a tape, and keeping what it wrote.
//!
//! A pty and nothing else. VHS itself drives a terminal through `ttyd` and
//! screenshots xterm.js canvases in headless Chromium, which in a container
//! produces zero frames, invokes no encoder, and exits 0 — a failure with
//! no error in it. It also puts a browser behind a tool whose README says
//! it needs none. A pty plus a terminal renderer does the same job from the
//! same tape.

use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

use crate::tape::Step;

/// Everything the terminal wrote, stamped.
pub struct Recording {
    pub events: Vec<(u64, Vec<u8>)>,
    /// Where each run of steps ended, in milliseconds. One per span.
    pub boundaries: Vec<u64>,
    pub cols: u16,
    pub rows: u16,
}

pub struct Terminal {
    pub cols: u16,
    pub rows: u16,
    pub shell: String,
    /// How long to let the shell draw its first prompt before the tape
    /// starts. A prompt that arrives mid-`Type` puts the command halfway
    /// up the screen.
    pub settle_ms: u64,
    pub env: Vec<(String, String)>,
    /// Where the shell starts. `None` inherits, which is the directory the
    /// build was run from — usually the project, which is what a tape
    /// demonstrating the project expects.
    pub cwd: Option<String>,
}

/// Run `spans` — each a list of steps — as one session, noting where each
/// one ended.
///
/// One session, not one per span: the beats of a walkthrough continue one
/// another, and running each in its own terminal would restart the program
/// once per beat.
pub fn record(terminal: &Terminal, spans: &[Vec<Step>]) -> std::io::Result<Recording> {
    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize {
            rows: terminal.rows,
            cols: terminal.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(other)?;

    let mut command = CommandBuilder::new(&terminal.shell);
    // `--norc --noprofile` so a recording does not depend on whose machine
    // it was made on: a contributor's prompt, aliases and shell plugins are
    // not part of the script.
    command.args(["--norc", "--noprofile", "-i"]);
    command.env("TERM", "xterm-256color");
    command.env("PS1", "\\[\\033[32m\\]❯\\[\\033[0m\\] ");
    command.env("LINES", terminal.rows.to_string());
    command.env("COLUMNS", terminal.cols.to_string());
    for (key, value) in &terminal.env {
        command.env(key, value);
    }
    if let Some(cwd) = &terminal.cwd {
        command.cwd(cwd);
    }

    let mut child = pair.slave.spawn_command(command).map_err(other)?;
    // The slave end has to be dropped here or the reader never sees EOF.
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().map_err(other)?;
    let writer = pair.master.take_writer().map_err(other)?;

    // A thread, because the pty reader blocks and the driver has to keep a
    // clock. A recording that stops reading fills the pty buffer and the
    // program on the other end stops drawing.
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let pump = std::thread::spawn(move || {
        let mut buf = [0u8; 65_536];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                return;
            }
        }
    });

    let start = Instant::now();
    let mut events: Vec<(u64, Vec<u8>)> = Vec::new();
    let mut boundaries = Vec::new();

    let mut ctx = Draining {
        start,
        rx,
        writer,
        events: &mut events,
    };

    ctx.until(start + Duration::from_millis(terminal.settle_ms));

    for steps in spans {
        for step in steps {
            match step {
                Step::Type { text, per_char_ms } => {
                    for ch in text.chars() {
                        ctx.send(ch.to_string().as_bytes());
                        ctx.for_ms(*per_char_ms);
                    }
                }
                Step::Keys {
                    bytes,
                    count,
                    per_key_ms,
                } => {
                    for _ in 0..*count {
                        ctx.send(bytes);
                        ctx.for_ms(*per_key_ms);
                    }
                }
                Step::Sleep(ms) => ctx.for_ms(*ms),
                // The screen going still is the closest thing to "the
                // command finished" that can be measured without owning
                // the prompt.
                Step::Quiet { timeout_ms } => {
                    let deadline = Instant::now() + Duration::from_millis(*timeout_ms);
                    loop {
                        let last =
                            ctx.until((Instant::now() + Duration::from_millis(250)).min(deadline));
                        if Instant::now() >= deadline
                            || last.elapsed() >= Duration::from_millis(250)
                        {
                            break;
                        }
                    }
                }
                Step::Mark => {}
            }
        }
        boundaries.push(start.elapsed().as_millis() as u64);
    }

    // A moment of stillness at the end, so the last frame is not caught
    // mid-redraw.
    ctx.for_ms(200);
    drop(ctx);
    if let Some(last) = boundaries.last_mut() {
        *last = start.elapsed().as_millis() as u64;
    }

    let _ = child.kill();
    let _ = child.wait();
    drop(pair.master);
    let _ = pump.join();

    Ok(Recording {
        events,
        boundaries,
        cols: terminal.cols,
        rows: terminal.rows,
    })
}

/// The driver's hands: a clock, a channel of output, and somewhere to
/// write. One struct because the loop has to do all three at once —
/// waiting without reading fills the pty buffer and the program on the
/// other end stops drawing.
struct Draining<'a> {
    start: Instant,
    rx: Receiver<Vec<u8>>,
    writer: Box<dyn Write + Send>,
    events: &'a mut Vec<(u64, Vec<u8>)>,
}

impl Draining<'_> {
    /// Read whatever arrives until `until`, and say when the last of it
    /// did — which is what "the screen went still" is measured from.
    fn until(&mut self, until: Instant) -> Instant {
        let mut last = Instant::now();
        loop {
            let now = Instant::now();
            if now >= until {
                return last;
            }
            match self.rx.recv_timeout(until - now) {
                Ok(chunk) => {
                    answer_queries(&mut self.writer, &chunk);
                    last = Instant::now();
                    self.events
                        .push((self.start.elapsed().as_millis() as u64, chunk));
                }
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return last,
            }
        }
    }

    fn for_ms(&mut self, ms: u64) {
        self.until(Instant::now() + Duration::from_millis(ms));
    }

    fn send(&mut self, bytes: &[u8]) {
        let _ = self.writer.write_all(bytes);
        let _ = self.writer.flush();
    }
}

/// Answer the questions a real terminal would answer.
///
/// A TUI asks what colour the terminal is (OSC 10/11) and what it can do
/// (DA1) before drawing, and waits out its own timeout when nobody
/// replies — a second of the recording spent on a question that was never
/// going to be answered. A program that turns focus reporting on (DECSET
/// 1004) may also skip its refresh while it believes it is in a background
/// window, which is a spinner that never resolves.
fn answer_queries(writer: &mut Box<dyn Write + Send>, chunk: &[u8]) {
    let text = String::from_utf8_lossy(chunk);
    let mut reply = Vec::new();
    if text.contains("\x1b]11;?") {
        reply.extend_from_slice(b"\x1b]11;rgb:0b0b/0d0d/1010\x1b\\");
    }
    if text.contains("\x1b]10;?") {
        reply.extend_from_slice(b"\x1b]10;rgb:d7d7/dede/e7e7\x1b\\");
    }
    if text.contains("\x1b[c") || text.contains("\x1b[0c") {
        reply.extend_from_slice(b"\x1b[?62;1;6;22c");
    }
    if text.contains("\x1b[?1004h") {
        reply.extend_from_slice(b"\x1b[I");
    }
    if !reply.is_empty() {
        let _ = writer.write_all(&reply);
        let _ = writer.flush();
    }
}

fn other(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}
