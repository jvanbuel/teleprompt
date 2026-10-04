//! The window: the welcome page; the prompter, which is the page
//! `teleprompt prompt` serves, shown in WebKit; and session mode, a
//! terminal that `teleprompt record` runs in.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gio, glib};
use teleprompt_gtk::launch::{LaunchEvent, LaunchRequest, ServerProcess};
use teleprompt_gtk::tools::{self, Tool};
use vte4::prelude::*;
use webkit::prelude::*;

use super::config::Config;
use super::session::SessionPage;
use super::tally::{Status, Tally};

/// Starts and stops recording, in every mode.
const RECORD_KEY: &str = "Ctrl ⇧ Space";

/// What background threads tell the window, tagged with the launch they
/// belong to so a stale one is dropped.
enum Event {
    Launch(u64, LaunchEvent),
    /// The tools a session can record with, for the session it was asked for.
    Tools(u64, Result<Vec<Tool>, String>),
}

/// What the prompter needs that it does not have, and how to get it.
const NO_BINARY: &str = "Set the teleprompt binary in Settings (Ctrl+,).";

pub struct Window {
    pub window: adw::ApplicationWindow,
    w: Widgets,
    model: RefCell<Model>,
    events: async_channel::Sender<Event>,
}

struct Widgets {
    stack: gtk::Stack,
    title: adw::WindowTitle,
    /// Session mode's Record: the page has its own.
    record: gtk::Button,
    record_label: gtk::Label,
    reopen: gtk::Button,
    /// Who reads, on the welcome page: the author, or a voice.
    narrator_voice: gtk::ToggleButton,
    loading_title: gtk::Label,
    loading_detail: gtk::Label,
    failed: adw::StatusPage,
    /// The prompter: the page the server serves.
    view: webkit::WebView,
    /// The welcome page's sample of the glass, which a narrow window drops.
    sample: gtk::Box,
    session: SessionPage,
    tally: Tally,
    toasts: adw::ToastOverlay,
}

#[derive(Default)]
struct Model {
    config: Config,
    generation: u64,
    server: Option<ServerProcess>,
    /// The server's origin, once it listens: the only one the page may
    /// load from, and use the microphone for.
    origin: Option<String>,
    /// The screen in windows of its own, which the page opened.
    screens: Vec<gtk::Window>,
    /// Session mode: the script a session is drafted into, and the
    /// `teleprompt record` recording it, once started.
    session: Option<Session>,
    status: Status,
}

struct Session {
    script: PathBuf,
    recording: Option<glib::Pid>,
    since: Option<Instant>,
    /// What it can record with, once `teleprompt record --tools` has said.
    tools: Vec<Tool>,
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
        this.connect_view(&this);
        if let Some(wav) = std::env::var_os("TELEPROMPT_MIC") {
            let countdown = this.config().countdown();
            hear_from_file(&this.w.view, std::path::Path::new(&wav), countdown);
        }
        // A narrow window keeps the controls: the key hints and the
        // welcome page's sample go.
        let narrow = adw::Breakpoint::new(
            adw::BreakpointCondition::parse("max-width: 900sp").expect("a condition"),
        );
        let hidden = false.to_value();
        narrow.add_setter(this.w.tally.keys(), "visible", Some(&hidden));
        narrow.add_setter(&this.w.sample, "visible", Some(&hidden));
        this.window.add_breakpoint(narrow);
        this.window.set_size_request(560, 420);
        this.show_welcome();
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
        // The record key, Ctrl+Shift+Space, in session mode: wherever the
        // focus is, and never typed into the session's shell. Elsewhere it
        // goes on to the page, whose key it is too.
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
        self.w.record.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.record_key();
            }
        });
        let weak = Rc::downgrade(this);
        self.w.narrator_voice.connect_toggled(move |voice| {
            if let Some(this) = weak.upgrade() {
                let mut config = this.config();
                config.voice_reads = voice.is_active();
                this.set_config(config);
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

    /// The page: shown once it has loaded, given the microphone and
    /// nothing else, and its monitor opened in a window of its own.
    fn connect_view(&self, this: &Rc<Self>) {
        let weak = Rc::downgrade(this);
        self.w.view.connect_load_changed(move |view, event| {
            let Some(this) = weak.upgrade() else { return };
            let ours = this
                .model
                .borrow()
                .origin
                .as_ref()
                .is_some_and(|o| view.uri().is_some_and(|u| u.starts_with(o.as_str())));
            if event == webkit::LoadEvent::Finished && ours {
                this.w.stack.set_visible_child_name("ready");
                this.w.view.grab_focus();
                // Named for the window switcher, once there is a script to
                // read: what a recording of the app waits for.
                let name = this.w.title.title();
                this.window.set_title(Some(&format!("{name} — Teleprompt")));
            }
        });
        let weak = Rc::downgrade(this);
        self.w.view.connect_load_failed(move |view, _, uri, error| {
            let Some(this) = weak.upgrade() else {
                return false;
            };
            // Leaving a page for the next is no failure.
            if error.matches(webkit::NetworkError::Cancelled) || view.uri().as_deref() != Some(uri)
            {
                return false;
            }
            this.shutdown();
            this.show_failed(&[format!("Could not show the prompter: {error}")]);
            true
        });
        let weak = Rc::downgrade(this);
        self.w
            .view
            .connect_permission_request(move |view, request| {
                let allowed = weak
                    .upgrade()
                    .is_some_and(|this| this.allows(view, request));
                if allowed {
                    request.allow();
                } else {
                    request.deny();
                }
                true
            });
        // The page's own controls, not the browser's: no Reload, no Back.
        self.w.view.connect_context_menu(|_, _, _| true);
        let weak = Rc::downgrade(this);
        self.w.view.connect_create(move |view, _| {
            let screen = webkit::WebView::builder().related_view(view).build();
            if let Some(this) = weak.upgrade() {
                this.screen_window(&screen);
            }
            screen.upcast()
        });
    }

    /// The microphone, for the page this server serves; nothing else, for
    /// any page.
    fn allows(&self, view: &webkit::WebView, request: &webkit::PermissionRequest) -> bool {
        let Some(media) = request.downcast_ref::<webkit::UserMediaPermissionRequest>() else {
            return false;
        };
        let origin = self.model.borrow().origin.clone();
        let ours = origin.is_some_and(|o| view.uri().is_some_and(|u| u.starts_with(&o)));
        ours && media.is_for_audio_device() && !media.is_for_video_device()
    }

    /// The page's monitor, which it opened, in a window of its own: for a
    /// second display.
    fn screen_window(&self, view: &webkit::WebView) {
        let window = gtk::Window::builder()
            .title("Teleprompt · Screen")
            .default_width(800)
            .default_height(520)
            .child(view)
            .build();
        view.connect_ready_to_show(glib::clone!(
            #[weak]
            window,
            move |_| window.present()
        ));
        view.connect_close(glib::clone!(
            #[weak]
            window,
            move |_| window.close()
        ));
        view.connect_context_menu(|_, _, _| true);
        self.model.borrow_mut().screens.push(window);
    }

    /// The record key and the Record button, in session mode: start or
    /// stop recording; false anywhere else.
    fn record_key(self: &Rc<Self>) -> bool {
        match self.session_recording() {
            Some(true) => {
                self.stop_session();
            }
            Some(false) => self.start_session(),
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
            model.session = Some(Session {
                script: script.clone(),
                recording: None,
                since: None,
                tools: Vec::new(),
            });
            model.status = Status::info(format!("New script: {}", file_name(&script)));
        }
        self.w.title.set_title(&file_name(&script));
        self.w.title.set_subtitle("Draft from a session");
        self.w.session.terminal.reset(true, true);
        self.w.session.set_tools(&[], None, |_| {});
        self.w.session.show_idle(&script);
        self.w.stack.set_visible_child_name("session");
        self.w.session.root.grab_focus();
        self.show_status();
        self.show_record();
        self.list_tools();
    }

    /// Asks `teleprompt` what it can record with, off the main thread.
    fn list_tools(&self) {
        let model = self.model.borrow();
        let Some(binary) = model.config.binary() else {
            return;
        };
        let (events, generation) = (self.events.clone(), model.generation);
        std::thread::spawn(move || {
            let _ = events.send_blocking(Event::Tools(generation, tools::list(&binary)));
        });
    }

    /// Offers the tools, the last one used picked. A `teleprompt` too old
    /// to list them records with its own default, and no choice is shown.
    fn tools_listed(self: &Rc<Self>, listed: Result<Vec<Tool>, String>) {
        let Ok(listed) = listed else { return };
        let chosen = {
            let model = self.model.borrow();
            tools::pick(&listed, model.config.record_with.as_deref()).map(|t| t.adapter.clone())
        };
        let weak = Rc::downgrade(self);
        self.w
            .session
            .set_tools(&listed, chosen.as_deref(), move |adapter| {
                if let Some(this) = weak.upgrade() {
                    let mut config = this.config();
                    config.record_with = Some(adapter.to_string());
                    this.set_config(config);
                }
            });
        let script = {
            let mut model = self.model.borrow_mut();
            model.session.as_mut().map(|session| {
                session.tools = listed;
                session.script.clone()
            })
        };
        if let Some(script) = script {
            self.w.session.show_idle(&script);
        }
    }

    /// Runs `teleprompt record` in the session's terminal.
    fn start_session(self: &Rc<Self>) {
        let (argv, dir) = {
            let model = self.model.borrow();
            let Some(session) = &model.session else {
                return;
            };
            let Some(binary) = model.config.binary() else {
                drop(model);
                return self.show_failed(&[NO_BINARY.into()]);
            };
            let Some(speech) = model.config.model() else {
                drop(model);
                return self.offer_setup_then(
                    &["drafts"],
                    Some(
                        "Drafting from a session listens to what you say, which needs the \
                     speech model; the punctuation model gives the draft capitals and \
                     full stops. Recording starts once they are installed.",
                    ),
                    |this| this.start_session(),
                );
            };
            (
                super::session::record_argv(
                    &binary,
                    &session.script,
                    self.w.session.chosen().as_deref(),
                    &speech,
                    model.config.punctuation().as_deref(),
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
        let in_terminal = {
            let mut model = self.model.borrow_mut();
            let chosen = self.w.session.chosen();
            let Some(session) = model.session.as_mut() else {
                return;
            };
            session.recording = Some(pid);
            session.since = Some(Instant::now());
            let in_terminal = session
                .tools
                .iter()
                .find(|t| Some(&t.adapter) == chosen.as_ref())
                .is_none_or(|t| t.in_terminal);
            model.status = Status::info("Recording: talk as you work");
            in_terminal
        };
        self.w.session.show_recording(in_terminal);
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
        self.model.borrow_mut().status = Status::info("Drafting the script…");
        self.w.session.show_drafting();
        self.show_status();
        true
    }

    /// `teleprompt record` ended: open the draft to read back, or say why
    /// there is none.
    fn session_ended(self: &Rc<Self>, status: i32) {
        let script = {
            let mut model = self.model.borrow_mut();
            let Some(session) = model.session.as_mut() else {
                return;
            };
            session.recording = None;
            session.since = None;
            session.script.clone()
        };
        if status == 0 && script.exists() {
            self.model.borrow_mut().session = None;
            self.open(script.clone());
            self.w.toasts.add_toast(adw::Toast::new(&format!(
                "Drafted {}: read it back, and re-take any line",
                file_name(&script)
            )));
            return;
        }
        self.model.borrow_mut().status = Status::error("The session did not become a script");
        self.w.session.show_failed();
        self.show_status();
        self.show_record();
    }

    fn handle(self: &Rc<Self>, event: Event) {
        let generation = self.model.borrow().generation;
        match event {
            Event::Launch(g, e) if g == generation => self.launched(e),
            Event::Tools(g, tools) if g == generation => self.tools_listed(tools),
            _ => {}
        }
    }

    /// Launches `teleprompt prompt` for `script`, replacing any server
    /// already running, and shows its page once it listens.
    pub fn open(self: &Rc<Self>, script: PathBuf) {
        self.shutdown();
        let mut model = self.model.borrow_mut();
        model.generation += 1;
        model.config.last_script = Some(script.clone());
        model.config.save();
        // A voice reads without a speech model; the author, with one.
        let voice = model.config.voice_reads;
        let (Some(binary), speech) = (model.config.binary(), model.config.model()) else {
            drop(model);
            return self.show_failed(&[NO_BINARY.into()]);
        };
        if speech.is_none() && !voice {
            drop(model);
            self.show_welcome();
            let again = script.clone();
            return self.offer_setup_then(
                &["prompt"],
                Some(
                    "Following your voice needs the speech model. The script opens \
                 once it is installed. Or let a voice read it: choose \u{201c}A voice \
                 reads\u{201d} on the welcome page.",
                ),
                move |this| this.open(again.clone()),
            );
        }
        let request = LaunchRequest {
            binary,
            script: script.clone(),
            model: speech.filter(|_| !voice),
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
        self.w.loading_title.set_label(if voice {
            "Opening the script"
        } else {
            "Loading the speech model"
        });
        self.w.loading_detail.set_label(&name);
        self.w.stack.set_visible_child_name("loading");
        self.show_record();
    }

    fn launched(&self, event: LaunchEvent) {
        match event {
            LaunchEvent::Listening(origin) => {
                // Settings the page takes in its address: it is in an
                // app, whose title bar names the script, and whether a
                // take counts down.
                let countdown = if self.config().countdown() {
                    ""
                } else {
                    "&countdown=0"
                };
                self.w
                    .view
                    .load_uri(&format!("{origin}/?shell=1{countdown}"));
                self.model.borrow_mut().origin = Some(origin);
            }
            LaunchEvent::Ended(reasons) => {
                self.shutdown();
                self.show_failed(&reasons);
            }
        }
    }

    fn show_status(&self) {
        let model = self.model.borrow();
        let recording = model
            .session
            .as_ref()
            .is_some_and(|s| s.recording.is_some());
        self.w.tally.set_status(&model.status);
        self.w.tally.set_on_air(recording);
        // Before recording, the session page itself says what the key does.
        self.w.tally.set_keys(if recording {
            &[(RECORD_KEY, "Stop")]
        } else {
            &[]
        });
    }

    fn show_time(&self) {
        let since = self.model.borrow().session.as_ref().and_then(|s| s.since);
        self.w
            .tally
            .set_time(since.map_or(0.0, |s| s.elapsed().as_secs_f64()));
    }

    /// Session mode's Record, and its tally bar: the prompter page has
    /// its own.
    fn show_record(&self) {
        let recording = self.session_recording();
        self.w.record.set_visible(recording.is_some());
        self.w.tally.root.set_visible(recording.is_some());
        let recording = recording.unwrap_or(false);
        self.w
            .record_label
            .set_label(if recording { "Stop" } else { "Record" });
        if recording {
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
        self.w
            .narrator_voice
            .set_active(self.model.borrow().config.voice_reads);
        self.w.stack.set_visible_child_name("welcome");
        self.window.set_title(Some("Teleprompt"));
        self.show_record();
    }

    /// Opens setup with `wanted` ticked, saying `why`: what a command
    /// found missing, to install there rather than fail.
    pub fn offer_setup(self: &Rc<Self>, wanted: &[&str], why: Option<&str>) {
        self.offer_setup_then(wanted, why, |this| this.list_tools());
    }

    /// As [`Self::offer_setup`], and `then` once something is installed:
    /// what the command was doing, carried on.
    fn offer_setup_then(
        self: &Rc<Self>,
        wanted: &[&str],
        why: Option<&str>,
        then: impl Fn(&Rc<Self>) + 'static,
    ) {
        let weak = Rc::downgrade(self);
        super::setup::present(
            self.file_dialog_parent(),
            self.config().binary(),
            wanted,
            why,
            move || {
                if let Some(this) = weak.upgrade() {
                    then(&this);
                }
            },
        );
    }

    fn show_failed(&self, reasons: &[String]) {
        self.w
            .failed
            .set_description(Some(&glib::markup_escape_text(&reasons.join("\n"))));
        self.w.stack.set_visible_child_name("failed");
        self.show_record();
    }

    /// The script in the author's own editor, for what the page does not
    /// edit: lines added, split or moved, and the shots' blocks.
    pub fn open_in_editor(&self) {
        let Some(script) = self.last_script() else {
            return;
        };
        let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(&script)));
        let toasts = self.w.toasts.clone();
        launcher.launch(Some(&self.window), gio::Cancellable::NONE, move |result| {
            if let Err(e) = result {
                toasts.add_toast(adw::Toast::new(&format!("Could not open the script: {e}")));
            }
        });
    }

    pub fn last_script(&self) -> Option<PathBuf> {
        self.model.borrow().config.last_script.clone()
    }

    /// Stops the session, the page and the server.
    pub fn shutdown(&self) {
        self.stop_session();
        let screens = {
            let mut model = self.model.borrow_mut();
            model.server = None;
            model.origin = None;
            std::mem::take(&mut model.screens)
        };
        // The page lets go of the microphone as it goes.
        self.w.view.load_uri("about:blank");
        for screen in screens {
            screen.close();
        }
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

/// An SVG from `apps/icons`, shown `height` pixels tall. Drawn here by
/// resvg at three times that size, so it stays sharp on a scaled screen:
/// GTK draws SVG only through a gdk-pixbuf loader not every desktop has.
fn picture(svg: &str, height: i32) -> Option<gtk::Widget> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).ok()?;
    let scale = 3.0 * height as f32 / tree.size().height();
    let size = tree.size().to_int_size().scale_by(scale)?;
    let mut pixmap = tiny_skia::Pixmap::new(size.width(), size.height())?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let stride = pixmap.width() as usize * 4;
    let texture = gtk::gdk::MemoryTexture::new(
        i32::try_from(pixmap.width()).ok()?,
        i32::try_from(pixmap.height()).ok()?,
        gtk::gdk::MemoryFormat::R8g8b8a8Premultiplied,
        &glib::Bytes::from_owned(pixmap.take()),
        stride,
    );
    let width = height * texture.width() / texture.height();
    let picture = gtk::Picture::builder()
        .paintable(&texture)
        .can_shrink(true)
        .content_fit(gtk::ContentFit::Contain)
        .halign(gtk::Align::Start)
        .alternative_text("Teleprompt")
        .build();
    picture.set_size_request(width, height);
    // A picture asks for its texture's full width; the clamp holds it to
    // the size it is drawn at.
    Some(
        adw::Clamp::builder()
            .maximum_size(width)
            .tightening_threshold(width)
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Center)
            .child(&picture)
            .build()
            .upcast(),
    )
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

/// The prompter's view: the page plays a shot's clip without a click, and
/// asks for the microphone, which the window answers.
fn prompter_view() -> webkit::WebView {
    let view = webkit::WebView::new();
    if let Some(settings) = WebViewExt::settings(&view) {
        settings.set_enable_media_stream(true);
        settings.set_media_playback_requires_user_gesture(false);
        // A microphone where there is no sound server, for a test: WebKit's
        // own, which hears a tone.
        settings.set_enable_mock_capture_devices(std::env::var_os("TELEPROMPT_MOCK_MIC").is_some());
    }

    // Black while it loads, as the glass is.
    view.set_background_color(&gtk::gdk::RGBA::BLACK);
    view.set_vexpand(true);
    view.set_hexpand(true);
    view
}

/// `TELEPROMPT_MIC`: a WAV the page hears as its microphone, for a test or
/// a recorded demonstration of the app, from when a take starts: after the
/// count of three, if there is one, as a reader would wait for it. WebKit
/// hears only devices, so the page's own `getUserMedia` is replaced.
fn hear_from_file(view: &webkit::WebView, wav: &std::path::Path, countdown: bool) {
    let bytes = match std::fs::read(wav) {
        Ok(bytes) => bytes,
        Err(e) => return eprintln!("TELEPROMPT_MIC: cannot read {}: {e}", wav.display()),
    };
    let source = format!(
        r#"(() => {{
  const wav = Uint8Array.from(atob("{}"), (c) => c.charCodeAt(0)).buffer;
  // On the prototype: WebKit keeps no property set on the object itself.
  Object.defineProperty(MediaDevices.prototype, "getUserMedia", {{
    configurable: true,
    writable: true,
    value: async () => {{
      const ctx = new AudioContext();
      const reading = ctx.createBufferSource();
      reading.buffer = await ctx.decodeAudioData(wav.slice(0));
      const out = ctx.createMediaStreamDestination();
      reading.connect(out);
      reading.start(ctx.currentTime + {});
      return out.stream;
    }},
  }});
}})();"#,
        glib::base64_encode(&bytes),
        // The page's count, three beats of 650 ms, and a breath after it.
        if countdown { 2.4 } else { 0.4 }
    );
    if let Some(content) = view.user_content_manager() {
        content.add_script(&webkit::UserScript::new(
            &source,
            webkit::UserContentInjectedFrames::TopFrame,
            webkit::UserScriptInjectionTime::Start,
            &[],
            &[],
        ));
    }
}

impl Widgets {
    fn build() -> Self {
        let (welcome, reopen, sample, narrator_voice) = Self::welcome();
        let (loading, loading_title, loading_detail) = Self::loading();
        let failed = Self::failed();
        let view = prompter_view();
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(220)
            .build();
        stack.add_named(&welcome, Some("welcome"));
        stack.add_named(&loading, Some("loading"));
        stack.add_named(&failed, Some("failed"));
        stack.add_named(&view, Some("ready"));
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
            .tooltip_text("Record the session, or stop and draft it (Ctrl+Shift+Space)")
            .visible(false)
            .build();
        Self {
            stack,
            title: adw::WindowTitle::new("Teleprompt", ""),
            record,
            record_label,
            reopen,
            narrator_voice,
            loading_title,
            loading_detail,
            failed,
            view,
            sample,
            session,
            tally: Tally::new(),
            toasts: adw::ToastOverlay::new(),
        }
    }

    fn welcome() -> (gtk::Box, gtk::Button, gtk::Box, gtk::ToggleButton) {
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
        let sample = Self::glass_sample();
        page.append(&sample);
        column.append(&Self::lockup());
        column.append(
            &gtk::Label::builder()
                .label(
                    "Open a script and read it aloud: the words follow your voice, \
                     each shot plays as you reach it, and every line you finish is \
                     kept as a take. Or let a voice read it, and direct it line by \
                     line.",
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
        let (narrator, narrator_voice) = Self::narrator();
        column.append(&narrator);
        column.append(&buttons);
        column.append(
            &gtk::Button::builder()
                .label("Set up what teleprompt needs…")
                .action_name("app.setup")
                .halign(gtk::Align::Start)
                .css_classes(["flat", "setup-link"])
                .build(),
        );
        (page, reopen, sample, narrator_voice)
    }

    /// Who reads the script: two linked toggles, "I read" and "A voice
    /// reads". The voice's toggle, whose state is the choice.
    fn narrator() -> (gtk::Box, gtk::ToggleButton) {
        let me = gtk::ToggleButton::builder()
            .label("I read")
            .active(true)
            .css_classes(["narrator"])
            .build();
        let voice = gtk::ToggleButton::builder()
            .label("A voice reads")
            .group(&me)
            .css_classes(["narrator"])
            .build();
        let toggles = gtk::Box::builder()
            .css_classes(["linked"])
            .halign(gtk::Align::Start)
            .build();
        toggles.append(&me);
        toggles.append(&voice);
        let row = gtk::Box::builder().spacing(14).margin_top(4).build();
        row.append(
            &gtk::Label::builder()
                .label("Who narrates")
                .css_classes(["narrator-label"])
                .build(),
        );
        row.append(&toggles);
        (row, voice)
    }

    /// The logo beside the name, `apps/icons`, or the name alone if the
    /// image cannot be read.
    fn lockup() -> gtk::Widget {
        let svg = include_str!("../../../icons/teleprompt-lockup-dark.svg");
        picture(svg, 64).unwrap_or_else(|| {
            gtk::Label::builder()
                .label("Teleprompt")
                .xalign(0.0)
                .css_classes(["hero-title"])
                .build()
                .upcast()
        })
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
            (false, "<span alpha='40%'>Every take starts here.</span>"),
            (true, "<span alpha='40%'>The words follow</span> <span foreground='#ffb800' underline='single' underline_color='#ffb800'>your</span> voice,"),
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

    fn loading() -> (gtk::Box, gtk::Label, gtk::Label) {
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
        let title = gtk::Label::builder()
            .label("Loading the speech model")
            .css_classes(["loading-title"])
            .build();
        page.append(&title);
        let detail = gtk::Label::builder()
            .css_classes(["loading-detail"])
            .build();
        page.append(&detail);
        (page, title, detail)
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
        // The app's mark beside the script's name: in the chrome, never on
        // the glass, where it would show in the reflection.
        let title = gtk::Box::builder()
            .spacing(10)
            .halign(gtk::Align::Center)
            .build();
        if let Some(mark) = picture(include_str!("../../../icons/teleprompt.svg"), 28) {
            title.append(&mark);
            // The welcome page shows the whole logo already.
            let beside_logo =
                |stack: &gtk::Stack| stack.visible_child_name().as_deref() == Some("welcome");
            mark.set_visible(!beside_logo(&self.stack));
            self.stack.connect_visible_child_name_notify(move |stack| {
                mark.set_visible(!beside_logo(stack))
            });
        }
        title.append(&self.title);
        let header = adw::HeaderBar::builder().title_widget(&title).build();
        let open = gtk::Button::builder()
            .icon_name("document-open-symbolic")
            .action_name("app.open")
            .tooltip_text("Open script (Ctrl+O)")
            .build();
        labelled(&open, "Open script");
        header.pack_start(&open);
        let menu = gio::Menu::new();
        let section = |items: &[(&str, &str)]| {
            let part = gio::Menu::new();
            for (label, action) in items {
                part.append(Some(label), Some(action));
            }
            menu.append_section(None, &part);
        };
        section(&[
            ("Open in editor", "app.open-editor"),
            ("Draft from a session…", "app.session"),
        ]);
        section(&[
            ("Settings", "app.settings"),
            ("Set up teleprompt…", "app.setup"),
            ("Keyboard shortcuts", "app.shortcuts"),
            ("Quit", "app.quit"),
        ]);
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .tooltip_text("Menu")
            .primary(true)
            .build();
        labelled(&menu_button, "Menu");
        header.pack_end(&menu_button);
        header.pack_end(&self.record);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        self.toasts.set_child(Some(&self.stack));
        view.set_content(Some(&self.toasts));
        view.add_bottom_bar(&self.tally.root);
        view
    }
}

/// Names an icon-only control for a screen reader; its tooltip is only a
/// description.
fn labelled(widget: &impl IsA<gtk::Accessible>, label: &str) {
    widget.update_property(&[gtk::accessible::Property::Label(label)]);
}
