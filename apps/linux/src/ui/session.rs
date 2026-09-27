//! Session mode's page. The recording itself happens in the author's own
//! terminal (`teleprompt_gtk::terminal`); this says what is going on.

use std::path::{Path, PathBuf};

use gtk::prelude::*;

pub struct SessionPage {
    pub root: gtk::Box,
    keycap: gtk::Label,
    spinner: gtk::Spinner,
    title: gtk::Label,
    body: gtk::Label,
}

impl SessionPage {
    pub fn new() -> Self {
        let keycap = gtk::Label::builder()
            .label("Ctrl ⇧ Space")
            .css_classes(["keycap-large"])
            .halign(gtk::Align::Center)
            .build();
        let spinner = gtk::Spinner::builder()
            .width_request(28)
            .height_request(28)
            .build();
        let title = gtk::Label::builder()
            .css_classes(["hero-title"])
            .wrap(true)
            .justify(gtk::Justification::Center)
            .build();
        let body = gtk::Label::builder()
            .css_classes(["hero-body"])
            .wrap(true)
            .max_width_chars(52)
            .justify(gtk::Justification::Center)
            .build();
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(16)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .css_classes(["session-hint"])
            .build();
        card.append(&keycap);
        card.append(&spinner);
        card.append(&title);
        card.append(&body);
        let root = gtk::Box::builder()
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        root.append(&card);
        Self {
            root,
            keycap,
            spinner,
            title,
            body,
        }
    }

    fn show(&self, keycap: bool, busy: bool, title: &str, body: &str) {
        self.keycap.set_visible(keycap);
        self.spinner.set_visible(busy);
        self.spinner.set_spinning(busy);
        self.title.set_label(title);
        self.body.set_label(body);
    }

    /// Before recording: what the record key does, and where the draft goes.
    pub fn show_idle(&self, script: &Path) {
        self.show(
            true,
            false,
            "Press Ctrl+Shift+Space to start recording",
            &format!(
                "Your terminal opens, recording. Talk while you use it, as if showing \
                 someone, and press Ctrl+Shift+Space there, or exit the shell, to stop. \
                 The session becomes {}: what you said as its lines, what you typed as \
                 its tapes.",
                name(script)
            ),
        );
    }

    pub fn show_opening(&self) {
        self.show(false, true, "Opening your terminal", "");
    }

    pub fn show_recording(&self) {
        self.show(
            true,
            false,
            "Recording in your terminal",
            "Talk while you use it. Press Ctrl+Shift+Space there or here, or exit \
             the shell, to stop.",
        );
    }

    pub fn show_drafting(&self, script: &Path) {
        self.show(
            false,
            true,
            &format!("Drafting {}", name(script)),
            "Transcribing what you said, and placing what you typed.",
        );
    }

    pub fn show_failed(&self, why: &str) {
        self.show(
            false,
            false,
            "The session did not become a script",
            &format!("{why}\n\nPress Ctrl+Shift+Space to record again."),
        );
    }
}

/// What a session runs: `teleprompt record` for `script`, reporting to
/// `status`, replacing the script if it exists (the save dialog has asked),
/// with punctuation if there is a model.
pub fn record_argv(
    binary: &Path,
    script: &Path,
    model: &Path,
    punctuation: Option<&Path>,
    status: &Path,
) -> Vec<String> {
    let mut argv = vec![
        binary.display().to_string(),
        "record".into(),
        script.display().to_string(),
        "--model".into(),
        model.display().to_string(),
        "--status".into(),
        status.display().to_string(),
    ];
    if let Some(p) = punctuation {
        argv.extend(["--punctuation".into(), p.display().to_string()]);
    }
    if script.exists() {
        argv.push("--force".into());
    }
    argv
}

/// Where a session's shell starts: the project, the parent of `scripts/`.
pub fn project_dir(script: &Path) -> PathBuf {
    let dir = script.parent().unwrap_or(script);
    if dir.file_name().is_some_and(|n| n == "scripts") {
        dir.parent().unwrap_or(dir).to_path_buf()
    } else {
        dir.to_path_buf()
    }
}

fn name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}
