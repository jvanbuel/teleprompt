//! The script read by its voice: Play reads from the line you are on,
//! lighting each word as it is said and starting the shots as it reaches
//! them, as a reader's voice would. The lines the voice has yet to make
//! are made in the background, so Play rarely waits.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::glib;
use teleprompt_gtk::api::{Position, Script, ServerMessage};
use teleprompt_gtk::state::Status;
use teleprompt_gtk::voice;

use super::{Event, Window};

/// The voice reading, line by line.
pub(super) struct Reading {
    /// Where it began: shots cued before it do not play.
    from: Position,
    /// The line being read.
    line: usize,
    /// Only this line, as Listen reads it.
    only: bool,
    media: Option<gtk::MediaFile>,
    /// The file it plays, removed when it is done with.
    file: Option<PathBuf>,
    ticker: Option<glib::SourceId>,
    started: Instant,
}

/// Names each line's audio file afresh, so a line made again is never
/// played stale.
static FETCHED: AtomicU64 = AtomicU64::new(0);

impl Window {
    /// Whether the script's voice reads it, rather than the author.
    pub(super) fn voice_reads(&self) -> bool {
        self.model
            .borrow()
            .state
            .script
            .voice
            .as_ref()
            .is_some_and(|v| !v.listens)
    }

    pub(super) fn is_reading(&self) -> bool {
        self.model.borrow().reading.is_some()
    }

    /// Space and the Play button: read from the line you are on, or stop.
    pub fn play_or_stop(self: &Rc<Self>) {
        if self.is_reading() {
            return self.stop_reading(None);
        }
        let (at, lines) = {
            let model = self.model.borrow();
            (model.state.at.line, model.state.script.lines.len())
        };
        self.read_from(if at < lines { at } else { 0 }, false, false);
    }

    /// Reads from `line` to the end of the script, or only `line`; with
    /// `fresh`, has the voice make the line anew first.
    pub(super) fn read_from(self: &Rc<Self>, line: usize, only: bool, fresh: bool) {
        if !self.voice_reads() || self.model.borrow().client.is_none() {
            return;
        }
        self.halt();
        self.w.edit.set_active(false);
        self.w.glass.unsay();
        let from = Position { line, word: 0 };
        {
            let mut model = self.model.borrow_mut();
            model.scrub = None;
            model.state.playing = None;
            model.state.queue.clear();
            model.state.started.clear();
            model.state.status = Status::info(if only {
                format!("Reading line {}", line + 1)
            } else {
                format!("Reading from line {}", line + 1)
            });
            model.reading = Some(Reading {
                from,
                line,
                only,
                media: None,
                file: None,
                ticker: None,
                started: Instant::now(),
            });
        }
        self.reach(from);
        self.fetch_voice(line, fresh);
        self.show_status();
        self.show_record();
        self.w.glass.view.grab_focus();
    }

    /// Moves the reading to `at`, starting the shots it reaches.
    fn reach(&self, at: Position) {
        let (play, before) = {
            let model = self.model.borrow();
            let Some(reading) = &model.reading else {
                return;
            };
            let state = &model.state;
            (
                voice::due(&state.script, reading.from, at, &state.started),
                state.playing.clone(),
            )
        };
        let played = !play.is_empty();
        self.model
            .borrow_mut()
            .state
            .apply(ServerMessage::Reached { at, play });
        if played {
            self.render();
        } else {
            self.w.glass.show_position(&self.model.borrow().state);
        }
        if self.model.borrow().state.playing != before {
            self.play();
        }
    }

    /// Fetches line `line`'s audio, made anew if `fresh`; for a line not
    /// made before, the script after too, which then knows its words.
    pub(super) fn fetch_voice(&self, line: usize, fresh: bool) {
        let model = self.model.borrow();
        let (Some(client), Some(audio)) = (
            model.client.clone(),
            model
                .state
                .script
                .lines
                .get(line)
                .and_then(|l| l.audio.clone()),
        ) else {
            return;
        };
        let (events, generation) = (self.events.clone(), model.generation);
        let dir = glib::user_cache_dir().join("teleprompt/voice");
        let known = audio.ready && !fresh;
        std::thread::spawn(move || {
            let url = if fresh {
                format!("{}?fresh=1", audio.url)
            } else {
                audio.url
            };
            let result = client.fetch(&url).and_then(|wav| {
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let n = FETCHED.fetch_add(1, Ordering::Relaxed);
                let file = dir.join(format!("{}-{n}.wav", std::process::id()));
                std::fs::write(&file, wav).map_err(|e| e.to_string())?;
                let script = if known { None } else { Some(client.script()?) };
                Ok((file, script))
            });
            let _ = events.send_blocking(Event::Voice(generation, line, result));
        });
    }

    /// A line's audio fetched: played, if the reading is still on it.
    pub(super) fn voice_fetched(
        self: &Rc<Self>,
        line: usize,
        result: Result<(PathBuf, Option<Script>), String>,
    ) {
        let (file, script) = match result {
            Ok(fetched) => fetched,
            Err(e) => {
                return self.stop_reading(Some(Status::error(format!("The voice failed: {e}"))))
            }
        };
        let reading_it = self
            .model
            .borrow()
            .reading
            .as_ref()
            .is_some_and(|r| r.line == line && r.media.is_none());
        if let Some(script) = script {
            self.model.borrow_mut().state.script = script;
            self.render();
        }
        if !reading_it {
            let _ = std::fs::remove_file(&file);
            return;
        }
        let media = gtk::MediaFile::for_filename(&file);
        let weak = Rc::downgrade(self);
        media.connect_ended_notify(move |media| {
            if media.is_ended() {
                if let Some(this) = weak.upgrade() {
                    this.line_read();
                }
            }
        });
        media.play();
        let weak = Rc::downgrade(self);
        let ticker =
            glib::timeout_add_local(Duration::from_millis(30), move || match weak.upgrade() {
                Some(this) => {
                    this.voice_tick();
                    glib::ControlFlow::Continue
                }
                None => glib::ControlFlow::Break,
            });
        if let Some(reading) = self.model.borrow_mut().reading.as_mut() {
            reading.media = Some(media);
            reading.file = Some(file);
            reading.ticker = Some(ticker);
        }
    }

    /// Lights the word the voice is saying.
    fn voice_tick(&self) {
        let at = {
            let model = self.model.borrow();
            let Some(reading) = &model.reading else {
                return;
            };
            let Some(media) = &reading.media else { return };
            let ms = u64::try_from(media.timestamp() / 1000).unwrap_or(0);
            let words = model
                .state
                .script
                .lines
                .get(reading.line)
                .and_then(|l| l.audio.as_ref())
                .and_then(|a| a.words.as_deref())
                .unwrap_or(&[]);
            Position {
                line: reading.line,
                word: voice::word_at(words, ms),
            }
        };
        if at != self.model.borrow().state.at {
            self.reach(at);
        }
    }

    /// The voice finished a line: on to the next, or done.
    fn line_read(self: &Rc<Self>) {
        let (next, lines, only) = {
            let model = self.model.borrow();
            let Some(reading) = &model.reading else {
                return;
            };
            (
                reading.line + 1,
                model.state.script.lines.len(),
                reading.only,
            )
        };
        self.drop_media();
        if only || next >= lines {
            let status = if only {
                Status::info(format!("Read line {next}"))
            } else {
                Status::info("Read to the end")
            };
            self.model.borrow_mut().state.at = Position {
                line: next.min(lines.saturating_sub(1)),
                word: 0,
            };
            return self.stop_reading(Some(status));
        }
        if let Some(reading) = self.model.borrow_mut().reading.as_mut() {
            reading.line = next;
        }
        self.reach(Position {
            line: next,
            word: 0,
        });
        self.fetch_voice(next, false);
    }

    fn drop_media(&self) {
        let mut model = self.model.borrow_mut();
        let Some(reading) = model.reading.as_mut() else {
            return;
        };
        if let Some(ticker) = reading.ticker.take() {
            ticker.remove();
        }
        if let Some(media) = reading.media.take() {
            media.pause();
        }
        if let Some(file) = reading.file.take() {
            let _ = std::fs::remove_file(file);
        }
    }

    /// Stops the voice where it is, saying `status` or that it stopped.
    pub(super) fn stop_reading(&self, status: Option<Status>) {
        if !self.is_reading() {
            return;
        }
        self.halt();
        self.model.borrow_mut().state.status =
            status.unwrap_or_else(|| Status::info("Stopped. Space reads on from here"));
        self.w.glass.show_position(&self.model.borrow().state);
        self.show_status();
        self.show_record();
    }

    /// Ends any reading, quietly.
    pub(super) fn halt(&self) {
        self.drop_media();
        self.model.borrow_mut().reading = None;
    }

    /// Seconds into the reading, for the tally's clock.
    pub(super) fn reading_time(&self) -> Option<f64> {
        self.model
            .borrow()
            .reading
            .as_ref()
            .map(|r| r.started.elapsed().as_secs_f64())
    }

    /// Has the voice make the lines it has yet to, one after another, in
    /// the background; once at a time.
    pub(super) fn voice_unmade(&self) {
        if !self.voice_reads() {
            return;
        }
        let model = self.model.borrow();
        if model
            .voicing
            .as_ref()
            .is_some_and(|v| v.load(Ordering::SeqCst))
        {
            return;
        }
        if voice::unmade(&model.state.script).is_empty() {
            return;
        }
        let Some(client) = model.client.clone() else {
            return;
        };
        let running = Arc::new(AtomicBool::new(true));
        let (events, generation, flag) = (self.events.clone(), model.generation, running.clone());
        drop(model);
        self.model.borrow_mut().voicing = Some(running);
        std::thread::spawn(move || {
            let mut failed = std::collections::HashSet::new();
            while flag.load(Ordering::SeqCst) {
                let Ok(script) = client.script() else { break };
                let next = voice::unmade(&script)
                    .into_iter()
                    .find(|&l| !failed.contains(&l));
                let Some(line) = next else {
                    let _ = events.send_blocking(Event::Voicing(generation, Ok(script)));
                    break;
                };
                let url = &script.lines[line].audio.as_ref().expect("unmade").url;
                if let Err(e) = client.fetch(url) {
                    failed.insert(line);
                    let why = format!("The voice could not read line {}: {e}", line + 1);
                    let _ = events.send_blocking(Event::Voicing(generation, Err(why)));
                    continue;
                }
                if let Ok(script) = client.script() {
                    let _ = events.send_blocking(Event::Voicing(generation, Ok(script)));
                }
            }
            flag.store(false, Ordering::SeqCst);
        });
    }

    /// The background voicing's news: a script with another line made,
    /// or a line it could not make.
    pub(super) fn voicing(&self, news: Result<Script, String>) {
        match news {
            Ok(script) => {
                // Read as it stands while the voice reads; the marks follow.
                let reading = self.is_reading();
                self.model.borrow_mut().state.script = script;
                if reading {
                    self.w.glass.update_marks(&self.model.borrow().state);
                } else {
                    self.render();
                    self.show_voice_status();
                }
            }
            Err(why) => self.model.borrow_mut().state.status = Status::error(why),
        }
        self.show_status();
    }

    /// The voice's status, unless a line is being reworded, whose own
    /// says how to finish.
    pub(super) fn show_voice_status(&self) {
        if self.model.borrow().rewording.is_none() {
            let status = self.voice_status();
            self.model.borrow_mut().state.status = status;
        }
    }

    /// Who reads, how long the video runs, and how far the voice has got.
    pub(super) fn voice_status(&self) -> Status {
        let model = self.model.borrow();
        let script = &model.state.script;
        let name = script.voice.as_ref().map_or("", |v| v.name.as_str());
        let (made, all) = voice::made(script);
        if made < all {
            return Status::info(format!("{name} is reading the lines: {made} of {all}"));
        }
        match script.length_ms {
            Some(ms) => Status::info(format!(
                "{name} · {} long. Space reads from here; click a line to direct it",
                voice::clock(ms)
            )),
            None => Status::info(name.to_string()),
        }
    }
}
