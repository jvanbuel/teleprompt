//! Setting teleprompt up: what you want to do with it, each use with what
//! it still needs, and one Install that shows how it goes. What the uses
//! are and how each installs is `teleprompt setup`'s; this asks it.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use teleprompt_gtk::setup::{self, Step, Use};

/// What the install thread says, for the dialog to show.
enum Said {
    Uses(Result<Vec<Use>, String>),
    Step(Step),
    Finished(Result<(), String>),
}

struct Dialog {
    dialog: adw::Dialog,
    list: gtk::ListBox,
    install: gtk::Button,
    status: gtk::Label,
    progress: gtk::ProgressBar,
    /// Each use shown, with its check box: none for one installed.
    rows: RefCell<Vec<(Use, Option<gtk::CheckButton>)>>,
    binary: PathBuf,
    /// Ticked when the list arrives: the uses a command found missing.
    wanted: Vec<String>,
    said: async_channel::Sender<Said>,
    /// Called once something is installed, for the window to look again.
    on_installed: Box<dyn Fn()>,
}

/// Shows the dialog over `parent`, asking `binary` what it can set up.
/// `wanted` are ticked from the start, and `why` says why it opened, when
/// a command opened it.
pub fn present(
    parent: &impl IsA<gtk::Widget>,
    binary: Option<PathBuf>,
    wanted: &[&str],
    why: Option<&str>,
    on_installed: impl Fn() + 'static,
) {
    let Some(binary) = binary else {
        let alert = adw::AlertDialog::new(
            Some("Where is teleprompt?"),
            Some("Choose the teleprompt binary in Settings first: it is what sets up the rest."),
        );
        alert.add_response("ok", "OK");
        alert.present(Some(parent));
        return;
    };
    let (said, heard) = async_channel::unbounded();
    let this = Rc::new(build(binary, wanted, why, said, Box::new(on_installed)));
    this.dialog.present(Some(parent));
    this.look();
    // Closing the dialog closes the channel, which ends the loop that
    // keeps it; an install under way goes on, unwatched.
    let said = this.said.clone();
    this.dialog.connect_closed(move |_| {
        said.close();
    });
    glib::spawn_future_local(async move {
        while let Ok(said) = heard.recv().await {
            this.hear(said);
        }
    });
}

fn build(
    binary: PathBuf,
    wanted: &[&str],
    why: Option<&str>,
    said: async_channel::Sender<Said>,
    on_installed: Box<dyn Fn()>,
) -> Dialog {
    let intro = gtk::Label::builder()
        .label(why.unwrap_or(
            "Choose what you want to do. Teleprompt ships no tools or models of its \
             own: each installs with your own package manager, under its own license.",
        ))
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dim-label"])
        .build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    list.append(
        &adw::ActionRow::builder()
            .title("Looking at what is installed…")
            .build(),
    );
    let status = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .selectable(true)
        .visible(false)
        .build();
    let progress = gtk::ProgressBar::builder().visible(false).build();
    let install = gtk::Button::builder()
        .label("Install")
        .sensitive(false)
        .halign(gtk::Align::End)
        .css_classes(["pill", "suggested-action"])
        .build();
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .margin_top(12)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build();
    body.append(&intro);
    body.append(&list);
    body.append(&progress);
    body.append(&status);
    body.append(&install);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&body)
            .build(),
    ));
    let dialog = adw::Dialog::builder()
        .title("Set up teleprompt")
        .content_width(620)
        .content_height(700)
        .child(&view)
        .build();
    Dialog {
        dialog,
        list,
        install,
        status,
        progress,
        rows: RefCell::default(),
        binary,
        wanted: wanted.iter().map(|w| (*w).to_string()).collect(),
        said,
        on_installed,
    }
}

impl Dialog {
    /// Asks teleprompt what it can set up, off the main thread.
    fn look(self: &Rc<Self>) {
        let (said, binary) = (self.said.clone(), self.binary.clone());
        std::thread::spawn(move || {
            let _ = said.send_blocking(Said::Uses(setup::uses(&binary)));
        });
        let weak = Rc::downgrade(self);
        self.install.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.start();
            }
        });
    }

    fn hear(self: &Rc<Self>, said: Said) {
        match said {
            Said::Uses(Ok(uses)) => self.show(uses),
            Said::Uses(Err(why)) => self.fail(&why),
            Said::Step(step) => self.step(&step),
            Said::Finished(Ok(())) => {
                self.progress.set_visible(false);
                self.say("Installed.");
                (self.on_installed)();
                // As the machine is now.
                let (said, binary) = (self.said.clone(), self.binary.clone());
                std::thread::spawn(move || {
                    let _ = said.send_blocking(Said::Uses(setup::uses(&binary)));
                });
            }
            Said::Finished(Err(why)) => {
                self.progress.set_visible(false);
                self.fail(&why);
                self.list.set_sensitive(true);
                self.update_button();
            }
        }
    }

    /// The uses, each a row: ticked if wanted, done if installed, greyed
    /// with why if this teleprompt cannot do it.
    fn show(self: &Rc<Self>, uses: Vec<Use>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let mut rows = Vec::new();
        for u in uses {
            let row = adw::ActionRow::builder()
                .title(&u.label)
                .subtitle(subtitle(&u))
                .build();
            let check = (u.available && !u.installed).then(|| {
                let check = gtk::CheckButton::builder()
                    .active(self.wanted.contains(&u.name))
                    .valign(gtk::Align::Center)
                    .build();
                row.add_prefix(&check);
                row.set_activatable_widget(Some(&check));
                let weak = Rc::downgrade(self);
                check.connect_toggled(move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.update_button();
                    }
                });
                check
            });
            if u.installed {
                row.add_suffix(&gtk::Image::from_icon_name("object-select-symbolic"));
            }
            row.set_sensitive(u.available);
            self.list.append(&row);
            rows.push((u, check));
        }
        *self.rows.borrow_mut() = rows;
        self.list.set_sensitive(true);
        self.update_button();
    }

    /// The uses ticked.
    fn chosen(&self) -> Vec<Use> {
        self.rows
            .borrow()
            .iter()
            .filter(|(_, check)| check.as_ref().is_some_and(gtk::CheckButton::is_active))
            .map(|(u, _)| u.clone())
            .collect()
    }

    /// "Install", with how much it downloads.
    fn update_button(&self) {
        let chosen = self.chosen();
        let mb: u32 = distinct_downloads(&chosen);
        self.install.set_sensitive(!chosen.is_empty());
        self.install.set_label(&match mb {
            0 => "Install".to_string(),
            mb => format!("Install · {mb} MB"),
        });
    }

    fn start(self: &Rc<Self>) {
        let names: Vec<String> = self.chosen().into_iter().map(|u| u.name).collect();
        if names.is_empty() {
            return;
        }
        self.list.set_sensitive(false);
        self.install.set_sensitive(false);
        self.status.remove_css_class("error");
        self.say("Starting…");
        self.progress.set_visible(true);
        self.progress.set_fraction(0.0);
        let (said, binary) = (self.said.clone(), self.binary.clone());
        std::thread::spawn(move || {
            let steps = said.clone();
            let done = setup::install(&binary, &names, move |step| {
                let _ = steps.send_blocking(Said::Step(step));
            });
            let _ = said.send_blocking(Said::Finished(done));
        });
    }

    fn step(&self, step: &Step) {
        match step {
            Step::Started { tool } => {
                self.say(&format!("Installing {}…", spoken(tool)));
                self.progress.pulse();
            }
            Step::Downloading { tool, mb, of } => {
                let mb = (*mb).min(*of);
                self.say(&format!("Downloading {} · {mb} of {of} MB", spoken(tool)));
                self.progress
                    .set_fraction(f64::from(mb) / f64::from((*of).max(1)));
            }
            Step::Done { tool } => self.say(&format!("Installed {}.", spoken(tool))),
        }
    }

    fn say(&self, text: &str) {
        self.status.set_label(text);
        self.status.set_visible(true);
    }

    fn fail(&self, why: &str) {
        self.status.add_css_class("error");
        self.say(why);
    }
}

/// What a row says under its title.
fn subtitle(u: &Use) -> String {
    let password = u.missing().iter().any(|t| t.password);
    let mut text = u.state();
    if password && u.available && !u.installed {
        text.push_str(" · asks for your password");
    }
    text
}

/// What the chosen uses download, each model once though several need it.
fn distinct_downloads(chosen: &[Use]) -> u32 {
    let mut seen: Vec<&str> = Vec::new();
    let mut mb = 0;
    for t in chosen.iter().flat_map(Use::missing) {
        if let (Some(size), false) = (t.download_mb, seen.contains(&t.name.as_str())) {
            seen.push(&t.name);
            mb += size;
        }
    }
    mb
}

/// A tool as a sentence names it.
fn spoken(name: &str) -> &str {
    match name {
        "speech-model" => "the speech model",
        "punctuation-model" => "the punctuation model",
        "speaker-model" => "the speaker models",
        other => other,
    }
}
