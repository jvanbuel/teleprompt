//! The window: which page shows, and what the prompter does with the
//! server, the microphone and the screen.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::{mpsc, Arc, Mutex};

use adw::prelude::*;
use gtk::{gio, glib};
use teleprompt_gtk::api::{ClientMessage, Script, ServerMessage};
use teleprompt_gtk::launch::{LaunchEvent, LaunchRequest, ServerProcess};
use teleprompt_gtk::mic::{Mic, RATE};
use teleprompt_gtk::session::{Incoming, Outgoing, SessionClient};
use teleprompt_gtk::state::{PrompterState, Status};

use super::config::Config;
use super::mirror::Mirror;
use super::prompter::{self, Layout};

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
}

pub struct Window {
    pub window: adw::ApplicationWindow,
    w: Widgets,
    model: RefCell<Model>,
    events: async_channel::Sender<Event>,
}

struct Widgets {
    stack: gtk::Stack,
    reopen: gtk::Button,
    launching: adw::StatusPage,
    failed: adw::StatusPage,
    text: gtk::TextView,
    mirror: Mirror,
    screen: gtk::Stack,
    picture: gtk::Picture,
    slate: gtk::Label,
    status: gtk::Label,
    recording: gtk::Image,
    css: gtk::CssProvider,
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
    layout: Layout,
    text_size: f64,
    paused: bool,
    media: Option<gtk::MediaFile>,
    /// Screens in windows of their own.
    screens: Vec<gtk::Picture>,
}

impl Window {
    pub fn new(app: &adw::Application) -> Rc<Self> {
        let (events, received) = async_channel::unbounded();
        let w = Widgets::build();
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Teleprompt")
            .default_width(1200)
            .default_height(760)
            .content(&w.toolbar())
            .build();
        let this = Rc::new(Self {
            window,
            w,
            model: RefCell::new(Model {
                config: Config::load(),
                text_size: 44.0,
                ..Model::default()
            }),
            events,
        });
        this.connect(&this);
        this.apply_css();
        this.show_welcome();
        let weak = Rc::downgrade(&this);
        glib::spawn_future_local(async move {
            while let Ok(event) = received.recv().await {
                let Some(this) = weak.upgrade() else { break };
                this.handle(event);
            }
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
        self.w.text.add_controller(keys);

        let weak = Rc::downgrade(this);
        let click = gtk::GestureClick::new();
        click.connect_released(move |_, _, x, y| {
            let Some(this) = weak.upgrade() else { return };
            let (bx, by) = this.w.text.window_to_buffer_coords(
                gtk::TextWindowType::Widget,
                x as i32,
                y as i32,
            );
            let line = this
                .w
                .text
                .iter_at_location(bx, by)
                .and_then(|iter| this.model.borrow().layout.line_at(iter.offset()));
            if let Some(line) = line {
                this.take(line);
            }
        });
        self.w.text.add_controller(click);

        let weak = Rc::downgrade(this);
        self.w.reopen.connect_clicked(move |_| {
            let Some(this) = weak.upgrade() else { return };
            let last = this.model.borrow().config.last_script.clone();
            if let Some(last) = last {
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

    /// The prompter's own keys, while it has focus. Ctrl combinations are
    /// the app's accelerators and pass through.
    fn key(&self, key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> bool {
        use gtk::gdk::Key;
        if modifiers
            .intersects(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::ALT_MASK)
        {
            return false;
        }
        match key {
            Key::Return | Key::KP_Enter => self.keep(),
            Key::space => self.toggle_pause(),
            Key::m => self.toggle_mirror(),
            Key::s => self.toggle_screen(),
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
        let name = script
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.w
            .launching
            .set_description(Some(&format!("Starting teleprompt for {name}…")));
        self.w.stack.set_visible_child_name("launching");
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
        }
        self.render();
        self.w.stack.set_visible_child_name("ready");
        self.w.text.grab_focus();
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
                    self.show_position();
                }
                if self.model.borrow().state.playing != before {
                    self.play();
                }
                if stopped {
                    self.refresh();
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

    /// Starts a take at `line`, opening the microphone the first time.
    pub fn take(&self, line: usize) {
        if self.model.borrow().socket.is_none() {
            return;
        }
        if let Err(e) = self.open_mic() {
            self.model.borrow_mut().state.status = Status::error(e);
            return self.show_status();
        }
        {
            let mut model = self.model.borrow_mut();
            let mic = model.mic.as_ref().expect("opened");
            mic.set_sending(false);
            mic.flush();
            model.state.start_take(line);
            model.paused = false;
            let _ = model
                .socket
                .as_ref()
                .expect("connected")
                .send(Outgoing::Command(ClientMessage::Start {
                    from: line,
                    rate: RATE,
                }));
            model.mic.as_ref().expect("opened").set_sending(true);
        }
        self.render();
        self.play();
        self.show_status();
        self.w.text.grab_focus();
    }

    /// Ends the take; the server keeps the lines read in full.
    pub fn keep(&self) {
        let mut model = self.model.borrow_mut();
        let (Some(mic), Some(socket)) = (model.mic.as_ref(), model.socket.as_ref()) else {
            return;
        };
        mic.set_sending(false);
        let _ = socket.send(Outgoing::Audio(mic.flush()));
        let _ = socket.send(Outgoing::Command(ClientMessage::Stop));
        model.state.listening = false;
        model.paused = false;
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
            model.state.status = Status::info(if paused { "paused" } else { "listening" });
        }
        self.show_status();
    }

    fn open_mic(&self) -> Result<(), String> {
        let mut model = self.model.borrow_mut();
        if model.mic.is_some() {
            return Ok(());
        }
        let outlet = model.outlet.clone();
        model.mic = Some(Mic::open(move |samples| {
            if let Some(socket) = outlet.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
                let _ = socket.send(Outgoing::Audio(samples));
            }
        })?);
        Ok(())
    }

    /// Shows the shot that should be playing: fetches its clip, or shows
    /// why there is none.
    fn play(&self) {
        let model = self.model.borrow();
        let playing = model.state.playing.clone();
        let clip = model.state.playing_clip().map(str::to_string);
        drop(model);
        match (playing, clip) {
            (Some(shot), Some(path)) => self.fetch_clip(shot, path),
            (Some(shot), None) => self.show_slate(&format!(
                "{shot} was never captured: run teleprompt capture"
            )),
            (None, _) => self.show_slate(""),
        }
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
            Err(e) => return self.show_slate(&format!("{shot}: {e}")),
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
        media.play();
        self.w.picture.set_paintable(Some(&media));
        for screen in &self.model.borrow().screens {
            screen.set_paintable(Some(&media));
        }
        self.w.screen.set_visible_child_name("video");
        self.model.borrow_mut().media = Some(media);
    }

    fn clip_ended(&self) {
        self.model.borrow_mut().state.clip_ended();
        self.play();
    }

    fn show_slate(&self, text: &str) {
        let mut model = self.model.borrow_mut();
        if let Some(media) = model.media.take() {
            media.pause();
        }
        self.w.picture.set_paintable(None::<&gtk::gdk::Paintable>);
        for screen in &model.screens {
            screen.set_paintable(None::<&gtk::gdk::Paintable>);
        }
        let text = if text.is_empty() && model.state.started.is_empty() {
            "shots play here as you reach them"
        } else {
            text
        };
        self.w.slate.set_label(text);
        self.w.screen.set_visible_child_name("slate");
    }

    fn toggle_mirror(&self) {
        self.w.mirror.set_mirrored(!self.w.mirror.is_mirrored());
    }

    fn toggle_screen(&self) {
        self.w.screen.set_visible(!self.w.screen.is_visible());
    }

    fn resize(&self, by: f64) {
        {
            let mut model = self.model.borrow_mut();
            model.text_size = (model.text_size + by).max(16.0);
        }
        self.apply_css();
    }

    /// The screen in a window of its own, for a second display.
    pub fn screen_window(self: &Rc<Self>) {
        let picture = gtk::Picture::builder().css_classes(["screen"]).build();
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
        self.w.screen.set_visible(false);
        window.present();
    }

    fn render(&self) {
        let layout = prompter::render(&self.w.text.buffer(), &self.model.borrow().state);
        self.model.borrow_mut().layout = layout;
        self.show_position();
    }

    fn show_position(&self) {
        let model = self.model.borrow();
        prompter::show_position(&self.w.text, &model.layout, model.state.at);
    }

    fn show_status(&self) {
        let model = self.model.borrow();
        self.w.status.set_label(&model.state.status.text);
        if model.state.status.is_error {
            self.w.status.add_css_class("error");
        } else {
            self.w.status.remove_css_class("error");
        }
        self.w.recording.set_visible(model.state.listening);
    }

    fn apply_css(&self) {
        let size = self.model.borrow().text_size;
        self.w.css.load_from_string(&format!(
            "textview.prompter, textview.prompter > text {{ background-color: #000; color: #fff; \
             font-size: {size}px; font-weight: 500; }} \
             .screen {{ background-color: #000; }} \
             .slate {{ color: alpha(#fff, 0.5); }}"
        ));
    }

    pub fn show_welcome(&self) {
        let last = self.model.borrow().config.last_script.clone();
        self.w.reopen.set_visible(last.is_some());
        if let Some(name) = last.as_ref().and_then(|p| p.file_name()) {
            self.w
                .reopen
                .set_label(&format!("Reopen {}", name.to_string_lossy()));
        }
        self.w.stack.set_visible_child_name("welcome");
    }

    fn show_failed(&self, reasons: &[String]) {
        self.w
            .failed
            .set_description(Some(&glib::markup_escape_text(&reasons.join("\n"))));
        self.w.stack.set_visible_child_name("failed");
    }

    pub fn last_script(&self) -> Option<PathBuf> {
        self.model.borrow().config.last_script.clone()
    }

    /// Stops the microphone, the session and the server.
    pub fn shutdown(&self) {
        let mut model = self.model.borrow_mut();
        if let Some(socket) = model.socket.take() {
            let _ = socket.send(Outgoing::Close);
        }
        *model.outlet.lock().unwrap_or_else(|p| p.into_inner()) = None;
        model.mic = None;
        model.client = None;
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

impl Widgets {
    fn build() -> Self {
        let (text, mirror) = Self::prompter();
        let (screen, picture, slate) = Self::screen();
        let paned = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .start_child(&mirror)
            .end_child(&screen)
            .resize_end_child(true)
            .shrink_end_child(false)
            .position(720)
            .build();
        let (welcome, reopen) = Self::welcome();
        let spinner = gtk::Spinner::builder()
            .spinning(true)
            .width_request(32)
            .height_request(32)
            .build();
        let launching = adw::StatusPage::builder()
            .title("Starting")
            .child(&spinner)
            .build();
        let failed = Self::failed();
        let stack = gtk::Stack::new();
        stack.add_named(&welcome, Some("welcome"));
        stack.add_named(&launching, Some("launching"));
        stack.add_named(&failed, Some("failed"));
        stack.add_named(&paned, Some("ready"));

        let css = gtk::CssProvider::new();
        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().expect("a display"),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        Self {
            stack,
            reopen,
            launching,
            failed,
            text,
            mirror,
            screen,
            picture,
            slate,
            status: gtk::Label::builder().xalign(0.0).hexpand(true).build(),
            recording: gtk::Image::builder()
                .icon_name("media-record-symbolic")
                .css_classes(["error"])
                .visible(false)
                .build(),
            css,
        }
    }

    /// The script's text, in a scroller that can be mirrored.
    fn prompter() -> (gtk::TextView, Mirror) {
        let text = gtk::TextView::builder()
            .editable(false)
            .cursor_visible(false)
            .wrap_mode(gtk::WrapMode::Word)
            .left_margin(48)
            .right_margin(48)
            .top_margin(120)
            .bottom_margin(600)
            .pixels_below_lines(24)
            .css_classes(["prompter"])
            .focusable(true)
            .build();
        prompter::tags(&text.buffer());
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&text)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let mirror = Mirror::new(&scrolled);
        (text, mirror)
    }

    /// The clip playing, or a slate.
    fn screen() -> (gtk::Stack, gtk::Picture, gtk::Label) {
        let picture = gtk::Picture::builder().css_classes(["screen"]).build();
        let slate = gtk::Label::builder()
            .wrap(true)
            .css_classes(["slate"])
            .build();
        let screen = gtk::Stack::builder()
            .css_classes(["screen"])
            .width_request(320)
            .build();
        screen.add_named(&slate, Some("slate"));
        screen.add_named(&picture, Some("video"));
        (screen, picture, slate)
    }

    fn welcome() -> (adw::StatusPage, gtk::Button) {
        let reopen = gtk::Button::builder().css_classes(["pill"]).build();
        let open = gtk::Button::builder()
            .label("Open Script…")
            .action_name("app.open")
            .css_classes(["pill", "suggested-action"])
            .build();
        let buttons = gtk::Box::builder()
            .spacing(12)
            .halign(gtk::Align::Center)
            .build();
        buttons.append(&open);
        buttons.append(&reopen);
        let welcome = adw::StatusPage::builder()
            .title("Teleprompt")
            .description(
                "Open a script to read it. The prompter follows your voice, \
                 plays each shot as you reach it, and records your takes.",
            )
            .child(&buttons)
            .build();
        (welcome, reopen)
    }

    fn failed() -> adw::StatusPage {
        let buttons = gtk::Box::builder()
            .spacing(12)
            .halign(gtk::Align::Center)
            .build();
        for (label, action) in [("Try Again", "app.reopen"), ("Settings", "app.settings")] {
            buttons.append(
                &gtk::Button::builder()
                    .label(label)
                    .action_name(action)
                    .css_classes(["pill"])
                    .build(),
            );
        }
        adw::StatusPage::builder()
            .icon_name("dialog-warning-symbolic")
            .title("teleprompt stopped")
            .child(&buttons)
            .build()
    }

    fn toolbar(&self) -> adw::ToolbarView {
        let header = adw::HeaderBar::new();
        header.pack_start(
            &gtk::Button::builder()
                .icon_name("document-open-symbolic")
                .action_name("app.open")
                .tooltip_text("Open Script (Ctrl+O)")
                .build(),
        );
        let menu = gio::Menu::new();
        menu.append(Some("Take from the Top"), Some("app.take-top"));
        menu.append(Some("Keep Take"), Some("app.keep"));
        menu.append(Some("Screen in Its Own Window"), Some("app.screen-window"));
        menu.append(Some("Settings"), Some("app.settings"));
        header.pack_end(
            &gtk::MenuButton::builder()
                .icon_name("open-menu-symbolic")
                .menu_model(&menu)
                .build(),
        );

        let bar = gtk::Box::builder()
            .spacing(8)
            .margin_start(12)
            .margin_end(12)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        bar.append(&self.recording);
        bar.append(&self.status);
        bar.append(&gtk::Label::builder()
            .label("click a line to take it from there · ⏎ keep · space pause · m mirror · + − size · s screen")
            .css_classes(["dim-label", "caption"])
            .build());

        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&self.stack));
        view.add_bottom_bar(&bar);
        view
    }
}
