//! Session mode: a terminal that `teleprompt record` runs in, drafting a
//! script from what is typed and said while it records.

use std::path::{Path, PathBuf};

use gtk::prelude::*;
use vte4::prelude::*;

/// The session page: the terminal, and what to do before it records.
pub struct SessionPage {
    pub root: gtk::Overlay,
    pub terminal: vte4::Terminal,
    hint: gtk::Box,
    hint_title: gtk::Label,
    hint_body: gtk::Label,
}

impl SessionPage {
    pub fn new() -> Self {
        let terminal = vte4::Terminal::builder()
            .hexpand(true)
            .vexpand(true)
            .css_classes(["session-terminal"])
            .build();
        terminal.set_font(Some(&gtk::pango::FontDescription::from_string(
            "Monospace 13",
        )));
        set_palette(&terminal);
        let hint_title = gtk::Label::builder()
            .css_classes(["hero-title"])
            .wrap(true)
            .build();
        let hint_body = gtk::Label::builder()
            .css_classes(["hero-body"])
            .wrap(true)
            .max_width_chars(52)
            .justify(gtk::Justification::Center)
            .build();
        let keycap = gtk::Label::builder()
            .label("Ctrl ⇧ Space")
            .css_classes(["keycap-large"])
            .halign(gtk::Align::Center)
            .build();
        let hint = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(16)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .css_classes(["session-hint"])
            .build();
        hint.append(&keycap);
        hint.append(&hint_title);
        hint.append(&hint_body);
        let root = gtk::Overlay::builder().child(&terminal).build();
        root.add_overlay(&hint);
        Self {
            root,
            terminal,
            hint,
            hint_title,
            hint_body,
        }
    }

    /// Before recording: what the record key does, and where the draft goes.
    pub fn show_idle(&self, script: &Path) {
        self.show_hint(
            "Press Ctrl+Shift+Space to start recording",
            &format!(
                "Talk while you use the terminal, as if showing someone. Press it \
                 again, or exit the shell, to stop, and the session becomes {}: \
                 what you said as its lines, what you typed as its tapes.",
                name(script)
            ),
        );
    }

    pub fn show_failed(&self) {
        self.show_hint(
            "The recording did not become a script",
            "The terminal behind says why. Press Ctrl+Shift+Space to record again.",
        );
    }

    fn show_hint(&self, title: &str, body: &str) {
        self.hint_title.set_label(title);
        self.hint_body.set_label(body);
        self.hint.set_visible(true);
        self.terminal.set_opacity(0.25);
    }

    pub fn show_recording(&self) {
        self.hint.set_visible(false);
        self.terminal.set_opacity(1.0);
        self.terminal.grab_focus();
    }
}

/// The app's ink on its glass, and ANSI colours bright enough to read
/// there: VTE's own blue is near black on black.
fn set_palette(terminal: &vte4::Terminal) {
    let rgba = |hex: &str| gtk::gdk::RGBA::parse(hex).expect("a colour");
    let palette: Vec<gtk::gdk::RGBA> = [
        "#2c2f36", "#f28b82", "#3ddc84", "#ffb800", "#7aa2ff", "#d49cff", "#5ed4e0", "#d9dce1",
        "#5b606b", "#ff9e96", "#6ee7a6", "#ffcc4d", "#9dbbff", "#e2b8ff", "#8be4ec", "#f2f4f7",
    ]
    .iter()
    .map(|c| rgba(c))
    .collect();
    let refs: Vec<&gtk::gdk::RGBA> = palette.iter().collect();
    terminal.set_colors(Some(&rgba("#f2f4f7")), Some(&rgba("#000000")), &refs);
}

/// What a session runs: `teleprompt record` for `script`, replacing it if it
/// exists (the save dialog has asked), with punctuation if there is a model.
pub fn record_argv(
    binary: &Path,
    script: &Path,
    model: &Path,
    punctuation: Option<&Path>,
) -> Vec<String> {
    let mut argv = vec![
        binary.display().to_string(),
        "record".into(),
        script.display().to_string(),
        "--model".into(),
        model.display().to_string(),
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
