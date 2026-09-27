//! The window: which page shows, and what the prompter does with the
//! server, the microphone and the screen.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gio, glib};
use teleprompt_gtk::api::{ClientMessage, Script, ServerMessage};
use teleprompt_gtk::launch::{LaunchEvent, LaunchRequest, ServerProcess};
use teleprompt_gtk::mic::{Mic, RATE};
use teleprompt_gtk::session::{Incoming, Outgoing, SessionClient};
use teleprompt_gtk::state::{PrompterState, Status};
use vte4::prelude::*;

use super::config::Config;
use super::glass::Glass;
use super::monitor::Monitor;
use super::session::SessionPage;
use super::tally::Tally;

/// Starts and stops recording, in every mode.
const RECORD_KEY: &str = "Ctrl ⇧ Space";

/// What background threads tell the window, tagged with the launch they
/// belong to so a stale one is dropped.
enum Event {
    Launch(u64, LaunchEvent),
    Connected(
        u64,
        Result<(SessionClient, Script, mpsc::Sender<Outgoing>), String>,
    ),
    Incoming(u64, Incoming),
    Refreshed(u64, Script),
    Clip(u64, String, Result<PathBuf, String>),
    Level(f32),
}

pub struct Window {
    pub window: adw::ApplicationWindow,
    w: Widgets,
    model: RefCell<Model>,
    events: async_channel::Sender<Event>,
}

struct Widgets {
    stack: gtk::Stack,
    title: adw::WindowTitle,
    record: gtk::Button,
    record_label: gtk::Label,
    reopen: gtk::Button,
    loading_detail: gtk::Label,
    failed: adw::StatusPage,
    glass: Glass,
    monitor: Monitor,
    session: SessionPage,
    tally: Tally,
    toasts: adw::ToastOverlay,
    text_css: gtk::CssProvider,
}

#[derive(Default)]
struct Model {
    config: Config,
    state: PrompterState,
    generation: u64,
    server: Option<ServerProcess>,
    client: Option<SessionClient>,
    socket: Option<mpsc::Sender<Outgoing>>,
    /// The socket, for the microphone's thread.
    outlet: Arc<Mutex<Option<mpsc::Sender<Outgoing>>>>,
    mic: Option<Mic>,
    paused: bool,
    /// A countdown is running; a second take waits for it.
    counting: bool,
    /// The take's running time: what ran before a pause, and since.
    take_time: Duration,
    take_since: Option<Instant>,
    media: Option<gtk::MediaFile>,
    /// Screens in windows of their own.
    screens: Vec<gtk::Picture>,
    /// Session mode: the script a session is drafted into, and the
    /// `teleprompt record` recording it, once started.
    session: Option<Session>,
}

struct Session {
    script: PathBuf,
    recording: Option<glib::Pid>,
}

impl Window {
    pub fn new(app: &adw::Application) -> Rc<Self> {
        let (events, received) = async_channel::unbounded();
        let w = Widgets::build();
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Teleprompt")
            .default_width(1280)
            .default_height(780)
            .content(&w.layout())
            .build();
        let this = Rc::new(Self {
            window,
            w,
            model: RefCell::new(Model {
                config: Config::load(),
                ..Model::default()
            }),
            events,
        });
        this.connect(&this);
        this.w.glass.set_text_size(&this.w.text_css, 48.0);
        this.show_welcome();
        this.show_status();
        let weak = Rc::downgrade(&this);
        glib::spawn_future_local(async move {
            while let Ok(event) = received.recv().await {
                let Some(this) = weak.upgrade() else { break };
                this.handle(event);
            }
        });
        let weak = Rc::downgrade(&this);
        glib::timeout_add_local(Duration::from_millis(100), move || {
            let Some(this) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            this.show_time();
            glib::ControlFlow::Continue
        });
        this
    }

    fn connect(&self, this: &Rc<Self>) {
        let weak = Rc::downgrade(this);
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let handled = weak.upgrade().is_some_and(|this| this.key(key, modifiers));
            if handled {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        self.w.glass.view.add_controller(keys);

        // The record key, Ctrl+Shift+Space: starts and stops recording in
        // every mode, wherever the focus is, and is never typed into the
        // session's shell.
        let weak = Rc::downgrade(this);
        let record = gtk::EventControllerKey::new();
        record.set_propagation_phase(gtk::PropagationPhase::Capture);
        record.connect_key_pressed(move |_, key, _, modifiers| {
            let both = gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK;
            let pressed = key == gtk::gdk::Key::space && modifiers.contains(both);
            if pressed && weak.upgrade().is_some_and(|this| this.record_key()) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        self.window.add_controller(record);

        let weak = Rc::downgrade(this);
        let click = gtk::GestureClick::new();
        click.connect_released(move |_, _, x, y| {
            let Some(this) = weak.upgrade() else { return };
            if let Some(line) = this.w.glass.line_at(x, y) {
                this.take(line);
            }
        });
        self.w.glass.view.add_controller(click);

        let weak = Rc::downgrade(this);
        self.w.record.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.record_key();
            }
        });

        let weak = Rc::downgrade(this);
        self.w
            .session
            .terminal
            .connect_child_exited(move |_, status| {
                if let Some(this) = weak.upgrade() {
                    this.session_ended(status);
                }
            });

        let weak = Rc::downgrade(this);
        self.w.reopen.connect_clicked(move |_| {
            let Some(this) = weak.upgrade() else { return };
            if let Some(last) = this.last_script() {
                this.open(last);
            }
        });
        let weak = Rc::downgrade(this);
        self.window.connect_close_request(move |_| {
            if let Some(this) = weak.upgrade() {
                this.shutdown();
            }
            glib::Propagation::Proceed
        });
    }

    /// The record key and the Record button: start or stop recording in
    /// whichever mode is showing; false when there is nothing to record.
    fn record_key(self: &Rc<Self>) -> bool {
        match self.session_recording() {
            Some(true) => {
                self.stop_session();
            }
            Some(false) => self.start_session(),
            None if self.model.borrow().socket.is_some() => self.record_or_keep(),
            None => return false,
        }
        true
    }

    /// Whether session mode is recording; `None` outside session mode.
    fn session_recording(&self) -> Option<bool> {
        if self.w.stack.visible_child_name().as_deref() != Some("session") {
            return None;
        }
        let model = self.model.borrow();
        model.session.as_ref().map(|s| s.recording.is_some())
    }

    /// Session mode, drafting into `script`: the record key starts it.
    pub fn new_session(&self, script: PathBuf) {
        self.shutdown();
        {
            let mut model = self.model.borrow_mut();
            model.generation += 1;
            model.state = PrompterState::default();
            model.session = Some(Session {
                script: script.clone(),
                recording: None,
            });
            model.take_time = Duration::ZERO;
            model.take_since = None;
            model.state.status = Status::info(format!("New script: {}", file_name(&script)));
        }
        self.w.title.set_title(&file_name(&script));
        self.w.title.set_subtitle("Draft from a session");
        self.w.session.terminal.reset(true, true);
        self.w.session.show_idle(&script);
        self.w.stack.set_visible_child_name("session");
        self.w.session.root.grab_focus();
        self.show_status();
        self.show_record();
    }

    /// Runs `teleprompt record` in the session's terminal.
    fn start_session(self: &Rc<Self>) {
        let (argv, dir) = {
            let model = self.model.borrow();
            let Some(session) = &model.session else {
                return;
            };
            let (Some(binary), Some(speech)) = (model.config.binary(), model.config.model()) else {
                drop(model);
                return self.show_failed(&[
                    "Set the teleprompt binary and the speech model in Settings (Ctrl+,).".into(),
                ]);
            };
            (
                super::session::record_argv(
                    &binary,
                    &session.script,
                    &speech,
                    model.config.punctuation.as_deref(),
                ),
                super::session::project_dir(&session.script),
            )
        };
        let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
        let weak = Rc::downgrade(self);
        self.w.session.terminal.spawn_async(
            vte4::PtyFlags::DEFAULT,
            dir.to_str(),
            &argv,
            &[],
            glib::SpawnFlags::DEFAULT,
            || {},
            -1,
            gio::Cancellable::NONE,
            move |spawned| {
                let Some(this) = weak.upgrade() else { return };
                match spawned {
                    Ok(pid) => this.session_started(pid),
                    Err(e) => this.show_failed(&[format!("Could not start recording: {e}")]),
                }
            },
        );
    }

    fn session_started(&self, pid: glib::Pid) {
        {
            let mut model = self.model.borrow_mut();
            let Some(session) = model.session.as_mut() else {
                return;
            };
            session.recording = Some(pid);
            let model = &mut *model;
            model.take_time = Duration::ZERO;
            model.take_since = Some(Instant::now());
            model.state.listening = true;
            model.state.status = Status::info("Recording: talk as you work");
        }
        self.w.session.show_recording();
        self.show_status();
        self.show_record();
    }

    /// Stops a session under way; `teleprompt record` drafts the script as
    /// if the shell had exited. False when there is none.
    fn stop_session(&self) -> bool {
        let pid = {
            let model = self.model.borrow();
            model.session.as_ref().and_then(|s| s.recording)
        };
        let Some(pid) = pid else { return false };
        // SAFETY: a signal to the child this terminal spawned.
        unsafe {
            libc::kill(pid.0, libc::SIGTERM);
        }
        self.model.borrow_mut().state.status = Status::info("Drafting the script…");
        self.w.session.show_drafting();
        self.show_status();
        true
    }

    /// `teleprompt record` ended: open the draft to read back, or say why
    /// there is none.
    fn session_ended(self: &Rc<Self>, status: i32) {
        let script = {
            let mut model = self.model.borrow_mut();
            model.state.listening = false;
            let Some(session) = model.session.as_mut() else {
                return;
            };
            session.recording = None;
            session.script.clone()
        };
        self.stop_clock();
        if status == 0 && script.exists() {
            self.model.borrow_mut().session = None;
            self.open(script.clone());
            self.w.toasts.add_toast(adw::Toast::new(&format!(
                "Drafted {}: read it back, and re-take any line",
                file_name(&script)
            )));
            return;
        }
        self.model.borrow_mut().state.status = Status::error("The session did not become a script");
        self.w.session.show_failed();
        self.show_status();
        self.show_record();
    }

    /// A take from the line you are on, or
    /// keep the one under way.
    pub fn record_or_keep(self: &Rc<Self>) {
        if self.is_taking() {
            return self.keep();
        }
        let (at, lines) = {
            let model = self.model.borrow();
            (model.state.at.line, model.state.script.lines.len())
        };
        self.take(if at < lines { at } else { 0 });
    }

    /// The prompter's own keys, while it has focus. Ctrl combinations are
    /// the app's accelerators and pass through.
    fn key(self: &Rc<Self>, key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> bool {
        use gtk::gdk::Key;
        if modifiers
            .intersects(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::ALT_MASK)
        {
            return false;
        }
        match key {
            Key::Return | Key::KP_Enter => self.keep(),
            Key::p => self.toggle_pause(),
            Key::m => self
                .w
                .glass
                .root
                .set_mirrored(!self.w.glass.root.is_mirrored()),
            Key::s => self
                .w
                .monitor
                .root
                .set_visible(!self.w.monitor.root.is_visible()),
            Key::plus | Key::equal | Key::KP_Add => self.resize(4.0),
            Key::minus | Key::KP_Subtract => self.resize(-4.0),
            _ => return false,
        }
        true
    }

    fn handle(self: &Rc<Self>, event: Event) {
        let generation = self.model.borrow().generation;
        match event {
            Event::Launch(g, e) if g == generation => self.launched(e),
            Event::Connected(g, result) if g == generation => self.connected(result),
            Event::Incoming(g, incoming) if g == generation => self.incoming(incoming),
            Event::Refreshed(g, script) if g == generation => {
                self.model.borrow_mut().state.script = script;
                self.render();
            }
            Event::Clip(g, shot, clip) if g == generation => self.clip_fetched(&shot, clip),
            Event::Level(rms) => {
                if self.model.borrow().state.listening {
                    self.w.tally.set_level(rms);
                } else {
                    self.w.tally.set_level(0.0);
                }
            }
            _ => {}
        }
    }

    /// Launches `teleprompt prompt` for `script`, replacing any server
    /// already running.
    pub fn open(self: &Rc<Self>, script: PathBuf) {
        self.shutdown();
        let mut model = self.model.borrow_mut();
        model.generation += 1;
        model.config.last_script = Some(script.clone());
        model.config.save();
        let (Some(binary), Some(speech)) = (model.config.binary(), model.config.model()) else {
            drop(model);
            return self.show_failed(&[
                "Set the teleprompt binary and the speech model in Settings (Ctrl+,).".into(),
            ]);
        };
        let request = LaunchRequest {
            binary,
            script: script.clone(),
            model: speech,
            locale: model.config.locale(),
        };
        let (events, generation) = (self.events.clone(), model.generation);
        match ServerProcess::start(&request, move |e| {
            let _ = events.send_blocking(Event::Launch(generation, e));
        }) {
            Ok(server) => model.server = Some(server),
            Err(e) => {
                drop(model);
                return self
                    .show_failed(&[format!("Could not run {}: {e}", request.binary.display())]);
            }
        }
        drop(model);
        let name = file_name(&script);
        self.w.title.set_title(&name);
        self.w.title.set_subtitle(&project_name(&script));
        self.w.loading_detail.set_label(&name);
        self.w.stack.set_visible_child_name("loading");
        self.show_record();
    }

    fn launched(&self, event: LaunchEvent) {
        match event {
            LaunchEvent::Listening(origin) => {
                let (events, generation) = (self.events.clone(), self.model.borrow().generation);
                std::thread::spawn(move || {
                    let client = SessionClient::new(origin);
                    let result = client.script().and_then(|script| {
                        let incoming = events.clone();
                        let socket = client.open(move |i| {
                            let _ = incoming.send_blocking(Event::Incoming(generation, i));
                        })?;
                        Ok((client, script, socket))
                    });
                    let _ = events.send_blocking(Event::Connected(generation, result));
                });
            }
            LaunchEvent::Ended(reasons) => {
                self.shutdown();
                self.show_failed(&reasons);
            }
        }
    }

    fn connected(&self, result: Result<(SessionClient, Script, mpsc::Sender<Outgoing>), String>) {
        let (client, script, socket) = match result {
            Ok(connected) => connected,
            Err(e) => return self.show_failed(&[e]),
        };
        {
            let mut model = self.model.borrow_mut();
            *model.outlet.lock().unwrap_or_else(|p| p.into_inner()) = Some(socket.clone());
            model.client = Some(client);
            model.socket = Some(socket);
            model.state = PrompterState::default();
            model.state.load(script);
            // A clock left from a session, or another script, starts over.
            model.take_time = Duration::ZERO;
            model.take_since = None;
            model.state.status =
                Status::info("Press Ctrl+Shift+Space to record from here, or click a line");
        }
        // Opened now, so a take starts the moment it is asked for.
        if let Err(e) = self.open_mic() {
            self.model.borrow_mut().state.status = Status::error(e);
        }
        self.w.monitor.set_shots(&self.model.borrow().state);
        self.w
            .monitor
            .root
            .set_visible(!self.model.borrow().state.script.shots.is_empty());
        self.render();
        self.play();
        self.w.stack.set_visible_child_name("ready");
        self.w.glass.view.grab_focus();
        self.show_status();
        self.show_record();
    }

    fn incoming(&self, incoming: Incoming) {
        match incoming {
            Incoming::Message(message) => {
                let (before, stopped, played) = {
                    let mut model = self.model.borrow_mut();
                    let before = model.state.playing.clone();
                    let stopped = matches!(message, ServerMessage::Stopped { .. });
                    let played =
                        matches!(&message, ServerMessage::Reached { play, .. } if !play.is_empty());
                    model.state.apply(message);
                    (before, stopped, played)
                };
                if played {
                    self.render();
                } else {
                    self.w.glass.show_position(&self.model.borrow().state);
                }
                if self.model.borrow().state.playing != before {
                    self.play();
                }
                if stopped {
                    self.stop_clock();
                    self.refresh();
                    self.show_record();
                    let toast = adw::Toast::builder()
                        .title(self.model.borrow().state.status.text.as_str())
                        .timeout(4)
                        .build();
                    self.w.toasts.add_toast(toast);
                    self.play();
                }
            }
            Incoming::Closed(Some(why)) => {
                let mut model = self.model.borrow_mut();
                model.state.listening = false;
                model.state.status = Status::error(why);
            }
            Incoming::Closed(None) => {}
        }
        self.show_status();
    }

    /// Reads the script again, for which lines are now recorded.
    fn refresh(&self) {
        let model = self.model.borrow();
        let Some(client) = model.client.clone() else {
            return;
        };
        let (events, generation) = (self.events.clone(), model.generation);
        std::thread::spawn(move || {
            if let Ok(script) = client.script() {
                let _ = events.send_blocking(Event::Refreshed(generation, script));
            }
        });
    }

    fn is_taking(&self) -> bool {
        let model = self.model.borrow();
        model.state.listening || model.paused || model.counting
    }

    /// Starts a take at `line`, after a count of three if that is on.
    pub fn take(self: &Rc<Self>, line: usize) {
        {
            let model = self.model.borrow();
            if model.socket.is_none() || model.counting {
                return;
            }
        }
        if let Err(e) = self.open_mic() {
            self.model.borrow_mut().state.status = Status::error(e);
            return self.show_status();
        }
        {
            // Nothing heard before the take starts is part of it.
            let mut model = self.model.borrow_mut();
            let mic = model.mic.as_ref().expect("opened");
            mic.set_sending(false);
            mic.flush();
            model.state.listening = false;
            model.paused = false;
            model.state.at = teleprompt_gtk::api::Position { line, word: 0 };
        }
        self.w.glass.show_position(&self.model.borrow().state);
        if !self.model.borrow().config.countdown() {
            return self.begin_take(line);
        }
        self.model.borrow_mut().counting = true;
        self.model.borrow_mut().state.status =
            Status::info(format!("Recording from line {} in…", line + 1));
        self.show_status();
        self.show_record();
        self.count(line, 3);
    }

    fn count(self: &Rc<Self>, line: usize, n: u32) {
        if n == 0 {
            self.w.glass.show_countdown(None);
            self.model.borrow_mut().counting = false;
            return self.begin_take(line);
        }
        self.w.glass.show_countdown(Some(n));
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(650), move || {
            if let Some(this) = weak.upgrade() {
                if this.model.borrow().counting {
                    this.count(line, n - 1);
                }
            }
        });
    }

    fn begin_take(&self, line: usize) {
        {
            let mut model = self.model.borrow_mut();
            let Some(socket) = model.socket.clone() else {
                return;
            };
            model.state.start_take(line);
            model.state.status = Status::info(format!("Recording from line {}", line + 1));
            model.paused = false;
            model.take_time = Duration::ZERO;
            model.take_since = Some(Instant::now());
            let _ = socket.send(Outgoing::Command(ClientMessage::Start {
                from: line,
                rate: RATE,
            }));
            let started = model.mic.as_ref().expect("opened").start();
            if let Err(e) = started {
                model.state.status = Status::error(e);
            }
            model.mic.as_ref().expect("opened").set_sending(true);
        }
        self.render();
        self.play();
        self.show_status();
        self.show_record();
        self.w.glass.view.grab_focus();
    }

    /// Ends the take; the server keeps the lines read in full.
    pub fn keep(&self) {
        {
            let mut model = self.model.borrow_mut();
            if model.counting {
                model.counting = false;
                model.state.status =
                    Status::info("Press Ctrl+Shift+Space to record from here, or click a line");
                drop(model);
                self.w.glass.show_countdown(None);
                self.show_status();
                return self.show_record();
            }
            let (Some(mic), Some(socket)) = (model.mic.as_ref(), model.socket.as_ref()) else {
                return;
            };
            if !(model.state.listening || model.paused) {
                return;
            }
            mic.set_sending(false);
            let _ = socket.send(Outgoing::Audio(mic.flush()));
            let _ = socket.send(Outgoing::Command(ClientMessage::Stop));
            model.state.listening = false;
            model.paused = false;
            model.state.status = Status::info("Keeping the take…");
        }
        self.stop_clock();
        self.show_status();
        self.show_record();
    }

    fn toggle_pause(&self) {
        {
            let mut model = self.model.borrow_mut();
            if model.mic.is_none() || !(model.state.listening || model.paused) {
                return;
            }
            model.paused = !model.paused;
            let paused = model.paused;
            model.mic.as_ref().expect("checked").set_sending(!paused);
            model.state.listening = !paused;
            model.state.status = Status::info(if paused { "Paused" } else { "Recording" });
            if paused {
                if let Some(since) = model.take_since.take() {
                    model.take_time += since.elapsed();
                }
            } else {
                model.take_since = Some(Instant::now());
            }
        }
        self.show_status();
        self.show_record();
    }

    fn stop_clock(&self) {
        let mut model = self.model.borrow_mut();
        if let Some(since) = model.take_since.take() {
            model.take_time += since.elapsed();
        }
    }

    fn open_mic(&self) -> Result<(), String> {
        let mut model = self.model.borrow_mut();
        if model.mic.is_some() {
            return Ok(());
        }
        let outlet = model.outlet.clone();
        let levels = self.events.clone();
        model.mic = Some(Mic::open(
            move |samples| {
                if let Some(socket) = outlet.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
                    let _ = socket.send(Outgoing::Audio(samples));
                }
            },
            move |rms| {
                let _ = levels.try_send(Event::Level(rms));
            },
        )?);
        Ok(())
    }

    /// Shows the shot that should be playing: fetches its clip, or shows
    /// why there is none.
    fn play(&self) {
        let (playing, clip, started) = {
            let model = self.model.borrow();
            (
                model.state.playing.clone(),
                model.state.playing_clip().map(str::to_string),
                !model.state.started.is_empty(),
            )
        };
        self.w.monitor.update(&self.model.borrow().state);
        match (playing, clip) {
            (Some(shot), Some(path)) => self.fetch_clip(shot, path),
            (Some(shot), None) => self.show_slate(
                "This shot was never captured. Run teleprompt capture to record it.",
                Some(&shot),
                true,
            ),
            (None, _) => {
                let next = self.next_shot();
                let text = match &next {
                    Some((name, line)) => format!("Next: {name}, at line {line}"),
                    None if started => "Every shot has played.".to_string(),
                    None => "Each shot plays here as your reading reaches it.".to_string(),
                };
                self.show_slate(&text, None, false)
            }
        }
    }

    /// The next captured shot the reader has yet to reach, and its line.
    fn next_shot(&self) -> Option<(String, usize)> {
        let model = self.model.borrow();
        let state = &model.state;
        state
            .script
            .shots
            .iter()
            .find(|s| s.clip.is_some() && !state.started.contains(&s.shot) && s.at >= state.at)
            .map(|s| {
                (
                    s.shot.strip_suffix("#0").unwrap_or(&s.shot).to_string(),
                    s.at.line + 1,
                )
            })
    }

    fn fetch_clip(&self, shot: String, path: String) {
        let model = self.model.borrow();
        let Some(client) = model.client.clone() else {
            return;
        };
        let (events, generation) = (self.events.clone(), model.generation);
        let cache = glib::user_cache_dir().join("teleprompt/clips");
        std::thread::spawn(move || {
            let name = path.rsplit('/').next().unwrap_or("clip.mp4").to_string();
            let file = cache.join(name);
            let result = if file.is_file() {
                Ok(file)
            } else {
                client.fetch(&path).and_then(|bytes| {
                    std::fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
                    std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
                    Ok(file)
                })
            };
            let _ = events.send_blocking(Event::Clip(generation, shot, result));
        });
    }

    fn clip_fetched(self: &Rc<Self>, shot: &str, clip: Result<PathBuf, String>) {
        if self.model.borrow().state.playing.as_deref() != Some(shot) {
            return;
        }
        let path = match clip {
            Ok(path) => path,
            Err(e) => {
                return self.show_slate(&format!("The clip did not load: {e}"), Some(shot), true)
            }
        };
        let media = gtk::MediaFile::for_filename(&path);
        // The clip's sound would be heard by the microphone.
        media.set_muted(true);
        let weak: Weak<Self> = Rc::downgrade(self);
        media.connect_ended_notify(move |media| {
            if media.is_ended() {
                if let Some(this) = weak.upgrade() {
                    this.clip_ended();
                }
            }
        });
        let monitor = self.w.monitor.clone();
        media.connect_timestamp_notify(move |media| monitor.show_progress(media));
        media.play();
        self.w.monitor.show_clip(shot, &media);
        for screen in &self.model.borrow().screens {
            screen.set_paintable(Some(&media));
        }
        self.model.borrow_mut().media = Some(media);
    }

    fn clip_ended(&self) {
        self.model.borrow_mut().state.clip_ended();
        self.play();
    }

    fn show_slate(&self, text: &str, shot: Option<&str>, missing: bool) {
        let mut model = self.model.borrow_mut();
        if let Some(media) = model.media.take() {
            media.pause();
        }
        for screen in &model.screens {
            screen.set_paintable(None::<&gtk::gdk::Paintable>);
        }
        self.w.monitor.show_slate(text, shot, missing);
    }

    fn resize(&self, by: f64) {
        let size = (self.w.glass.text_size() + by).clamp(24.0, 120.0);
        self.w.glass.set_text_size(&self.w.text_css, size);
    }

    /// The screen in a window of its own, for a second display.
    pub fn screen_window(self: &Rc<Self>) {
        let picture = gtk::Picture::builder().css_classes(["monitor"]).build();
        if let Some(media) = &self.model.borrow().media {
            picture.set_paintable(Some(media));
        }
        let window = gtk::Window::builder()
            .title("Screen")
            .default_width(960)
            .default_height(540)
            .transient_for(&self.window)
            .child(&picture)
            .build();
        self.model.borrow_mut().screens.push(picture.clone());
        let weak = Rc::downgrade(self);
        window.connect_close_request(move |_| {
            if let Some(this) = weak.upgrade() {
                this.model.borrow_mut().screens.retain(|p| p != &picture);
            }
            glib::Propagation::Proceed
        });
        self.w.monitor.root.set_visible(false);
        window.present();
    }

    fn render(&self) {
        let model = self.model.borrow();
        self.w.glass.render(&model.state);
        self.w.monitor.update(&model.state);
    }

    fn show_status(&self) {
        let model = self.model.borrow();
        self.w.tally.set_status(&model.state.status);
        self.w.tally.set_on_air(model.state.listening);
        let session = self
            .w
            .stack
            .visible_child_name()
            .is_some_and(|n| n == "session");
        let keys: &[(&str, &str)] = if session {
            // Before recording, the page itself says what the key does.
            if model.state.listening {
                &[(RECORD_KEY, "Stop")]
            } else {
                &[]
            }
        } else if model.counting {
            &[(RECORD_KEY, "Cancel")]
        } else if model.paused {
            &[(RECORD_KEY, "Keep take"), ("P", "Resume")]
        } else if model.state.listening {
            &[(RECORD_KEY, "Keep take"), ("P", "Pause")]
        } else if model.socket.is_some() {
            &[
                (RECORD_KEY, "Record"),
                ("Ctrl T", "From the top"),
                ("M", "Mirror"),
            ]
        } else {
            &[]
        };
        self.w.tally.set_keys(keys);
        if !model.state.listening {
            self.w.tally.set_level(0.0);
        }
    }

    fn show_time(&self) {
        let model = self.model.borrow();
        let running = model.take_since.map_or(Duration::ZERO, |s| s.elapsed());
        self.w
            .tally
            .set_time((model.take_time + running).as_secs_f64());
    }

    /// The header's one action: record, or keep the take under way.
    fn show_record(&self) {
        if let Some(recording) = self.session_recording() {
            self.w.record.set_visible(true);
            self.w.tally.root.set_visible(true);
            self.w
                .record_label
                .set_label(if recording { "Stop" } else { "Record" });
            if recording {
                self.w.record.add_css_class("keep");
            } else {
                self.w.record.remove_css_class("keep");
            }
            return;
        }
        let taking = self.is_taking();
        let ready = self.model.borrow().socket.is_some();
        self.w.record.set_visible(ready);
        self.w.tally.root.set_visible(ready);
        self.w
            .record_label
            .set_label(if taking { "Keep take" } else { "Record" });
        if taking {
            self.w.record.add_css_class("keep");
        } else {
            self.w.record.remove_css_class("keep");
        }
    }

    pub fn show_welcome(&self) {
        let last = self.last_script();
        self.w.reopen.set_visible(last.is_some());
        if let Some(last) = &last {
            self.w
                .reopen
                .set_label(&format!("Reopen {}", file_name(last)));
        }
        self.w.stack.set_visible_child_name("welcome");
        self.show_record();
    }

    fn show_failed(&self, reasons: &[String]) {
        self.w
            .failed
            .set_description(Some(&glib::markup_escape_text(&reasons.join("\n"))));
        self.w.stack.set_visible_child_name("failed");
        self.show_record();
    }

    pub fn last_script(&self) -> Option<PathBuf> {
        self.model.borrow().config.last_script.clone()
    }

    /// Stops the microphone, the session and the server.
    pub fn shutdown(&self) {
        self.stop_session();
        let mut model = self.model.borrow_mut();
        if let Some(socket) = model.socket.take() {
            let _ = socket.send(Outgoing::Close);
        }
        *model.outlet.lock().unwrap_or_else(|p| p.into_inner()) = None;
        model.mic = None;
        model.client = None;
        model.counting = false;
        if let Some(media) = model.media.take() {
            media.pause();
        }
        model.server = None;
    }

    pub fn config(&self) -> Config {
        self.model.borrow().config.clone()
    }

    pub fn set_config(&self, config: Config) {
        config.save();
        self.model.borrow_mut().config = config;
    }

    pub fn file_dialog_parent(&self) -> &adw::ApplicationWindow {
        &self.window
    }
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The project a script belongs to, by its directory's name: the parent of
/// `scripts/`, or the script's own directory.
fn project_name(script: &std::path::Path) -> String {
    let dir = script.parent().unwrap_or(script);
    let dir = if dir.file_name().is_some_and(|n| n == "scripts") {
        dir.parent().unwrap_or(dir)
    } else {
        dir
    };
    file_name(dir)
}

impl Widgets {
    fn build() -> Self {
        let text_css = gtk::CssProvider::new();
        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().expect("a display"),
            &text_css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
        let glass = Glass::new();
        let monitor = Monitor::new();
        let paned = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .start_child(&glass.root)
            .end_child(&monitor.root)
            .resize_end_child(false)
            .shrink_end_child(false)
            .shrink_start_child(false)
            .wide_handle(false)
            .build();
        let (welcome, reopen) = Self::welcome();
        let (loading, loading_detail) = Self::loading();
        let failed = Self::failed();
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(220)
            .build();
        stack.add_named(&welcome, Some("welcome"));
        stack.add_named(&loading, Some("loading"));
        stack.add_named(&failed, Some("failed"));
        stack.add_named(&paned, Some("ready"));
        let session = SessionPage::new();
        stack.add_named(&session.root, Some("session"));
        let record_label = gtk::Label::new(Some("Record"));
        let record_inner = gtk::Box::builder().spacing(8).build();
        record_inner.append(
            &gtk::Box::builder()
                .css_classes(["record-dot"])
                .valign(gtk::Align::Center)
                .build(),
        );
        record_inner.append(&record_label);
        let record = gtk::Button::builder()
            .child(&record_inner)
            .css_classes(["record"])
            .tooltip_text("Record from the line you are on, or keep the take (Ctrl+Shift+Space)")
            .visible(false)
            .build();
        Self {
            stack,
            title: adw::WindowTitle::new("Teleprompt", ""),
            record,
            record_label,
            reopen,
            loading_detail,
            failed,
            glass,
            monitor,
            session,
            tally: super::tally::Tally::new(),
            toasts: adw::ToastOverlay::new(),
            text_css,
        }
    }

    fn welcome() -> (gtk::Box, gtk::Button) {
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(18)
            .valign(gtk::Align::Center)
            .build();
        let page = gtk::Box::builder()
            .spacing(64)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        page.append(&column);
        page.append(&Self::glass_sample());
        let page_for_column = page.clone();
        let page = column;
        page.append(
            &gtk::Label::builder()
                .label("Teleprompt")
                .xalign(0.0)
                .css_classes(["hero-title"])
                .build(),
        );
        page.append(
            &gtk::Label::builder()
                .label(
                    "Open a script and read it aloud. The words follow your voice, \
                     each shot plays as you reach it, and every line you finish is \
                     kept as a take. Or draft a new script by talking while you use \
                     a terminal.",
                )
                .wrap(true)
                .max_width_chars(46)
                .xalign(0.0)
                .css_classes(["hero-body"])
                .build(),
        );
        let open = gtk::Button::builder()
            .label("Open script…")
            .action_name("app.open")
            .css_classes(["pill", "primary"])
            .build();
        let reopen = gtk::Button::builder().css_classes(["pill"]).build();
        let session = gtk::Button::builder()
            .label("Draft from a session…")
            .action_name("app.session")
            .css_classes(["pill"])
            .build();
        let buttons = gtk::Box::builder().spacing(12).margin_top(10).build();
        buttons.append(&open);
        buttons.append(&reopen);
        buttons.append(&session);
        page.append(&buttons);
        (page_for_column, reopen)
    }

    /// A slice of the glass, to show what the prompter does before a
    /// script is open: the reading line, what is said, the next word.
    fn glass_sample() -> gtk::Box {
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .css_classes(["sample-glass"])
            .valign(gtk::Align::Center)
            .build();
        let lines = [
            (false, "<span alpha='32%'>Every take starts here.</span>"),
            (true, "<span alpha='32%'>The words follow</span> <span foreground='#ffb800' underline='single' underline_color='#ffb800'>your</span> voice,"),
            (false, "<span alpha='55%'>and the shots play</span>"),
            (false, "<span alpha='55%'>as you reach them.</span>"),
        ];
        for (current, markup) in lines {
            let row = gtk::Box::builder().spacing(14).build();
            let arrow = gtk::Label::builder()
                .label("▶")
                .css_classes(["sample-arrow"])
                .opacity(if current { 1.0 } else { 0.0 })
                .build();
            row.append(&arrow);
            row.append(
                &gtk::Label::builder()
                    .use_markup(true)
                    .label(markup)
                    .xalign(0.0)
                    .css_classes(["sample-text"])
                    .build(),
            );
            card.append(&row);
        }
        card
    }

    fn loading() -> (gtk::Box, gtk::Label) {
        let page = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        page.append(
            &gtk::Spinner::builder()
                .spinning(true)
                .width_request(28)
                .height_request(28)
                .margin_bottom(8)
                .build(),
        );
        page.append(
            &gtk::Label::builder()
                .label("Loading the speech model")
                .css_classes(["loading-title"])
                .build(),
        );
        let detail = gtk::Label::builder()
            .css_classes(["loading-detail"])
            .build();
        page.append(&detail);
        (page, detail)
    }

    fn failed() -> adw::StatusPage {
        let buttons = gtk::Box::builder()
            .spacing(12)
            .halign(gtk::Align::Center)
            .build();
        for (label, action, classes) in [
            ("Try again", "app.reopen", &["pill", "primary"][..]),
            ("Settings", "app.settings", &["pill"][..]),
        ] {
            buttons.append(
                &gtk::Button::builder()
                    .label(label)
                    .action_name(action)
                    .css_classes(classes.to_vec())
                    .build(),
            );
        }
        adw::StatusPage::builder()
            .icon_name("dialog-warning-symbolic")
            .title("The prompter stopped")
            .child(&buttons)
            .build()
    }

    fn layout(&self) -> adw::ToolbarView {
        let header = adw::HeaderBar::builder().title_widget(&self.title).build();
        header.pack_start(
            &gtk::Button::builder()
                .icon_name("document-open-symbolic")
                .action_name("app.open")
                .tooltip_text("Open script (Ctrl+O)")
                .build(),
        );
        let menu = gio::Menu::new();
        menu.append(Some("Record from the top"), Some("app.take-top"));
        menu.append(Some("Draft from a session…"), Some("app.session"));
        menu.append(Some("Screen in its own window"), Some("app.screen-window"));
        menu.append(Some("Settings"), Some("app.settings"));
        header.pack_end(
            &gtk::MenuButton::builder()
                .icon_name("open-menu-symbolic")
                .menu_model(&menu)
                .build(),
        );
        header.pack_end(&self.record);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        self.toasts.set_child(Some(&self.stack));
        view.set_content(Some(&self.toasts));
        view.add_bottom_bar(&self.tally.root);
        view
    }
}
