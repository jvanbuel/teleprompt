//! Session mode: a terminal that `teleprompt record` runs in, drafting a
//! script from what is typed and said while it records.

use std::path::{Path, PathBuf};

use gtk::prelude::*;
use vte4::prelude::*;

/// The session page: the terminal behind glass, and what to do before it
/// records.
pub struct SessionPage {
    pub root: gtk::Overlay,
    pub terminal: vte4::Terminal,
    frame: gtk::Box,
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
        // The desktop's monospace, a little larger, for being read back.
        terminal.set_font_scale(1.15);
        terminal.set_cursor_blink_mode(vte4::CursorBlinkMode::Off);
        set_palette(&terminal);
        let frame = gtk::Box::builder()
            .css_classes(["session-frame"])
            .margin_top(20)
            .margin_bottom(20)
            .margin_start(20)
            .margin_end(20)
            .overflow(gtk::Overflow::Hidden)
            .build();
        frame.append(&terminal);
        let hint_title = gtk::Label::builder()
            .css_classes(["session-title"])
            .wrap(true)
            .build();
        let hint_body = gtk::Label::builder()
            .css_classes(["session-body"])
            .wrap(true)
            .max_width_chars(46)
            .justify(gtk::Justification::Center)
            .build();
        // "Press [Ctrl ⇧ Space] to start recording", the key drawn as one.
        let press = gtk::Box::builder()
            .spacing(12)
            .halign(gtk::Align::Center)
            .build();
        press.append(
            &gtk::Label::builder()
                .label("Press")
                .css_classes(["session-title"])
                .build(),
        );
        press.append(
            &gtk::Label::builder()
                .label("Ctrl ⇧ Space")
                .css_classes(["keycap-large"])
                .valign(gtk::Align::Center)
                .build(),
        );
        press.append(&hint_title);
        let hint = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(16)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        hint.append(&press);
        hint.append(&hint_body);
        let root = gtk::Overlay::builder().child(&frame).build();
        root.add_overlay(&hint);
        Self {
            root,
            terminal,
            frame,
            hint,
            hint_title,
            hint_body,
        }
    }

    /// Before recording: what the record key does, and where the draft goes.
    pub fn show_idle(&self, script: &Path) {
        self.show_hint(
            "to start recording",
            &format!(
                "Work in the terminal and talk as if showing someone. Press it \
                 again to stop: what you said becomes the lines of {}, what \
                 you typed its tapes.",
                name(script)
            ),
        );
    }

    pub fn show_failed(&self) {
        self.show_hint(
            "to record again",
            "The recording did not become a script. The terminal says why.",
        );
        self.terminal.set_opacity(0.35);
    }

    fn show_hint(&self, title: &str, body: &str) {
        self.hint_title.set_label(title);
        self.hint_body.set_label(body);
        self.hint.set_visible(true);
        self.terminal.set_opacity(0.0);
        self.frame.remove_css_class("recording");
    }

    pub fn show_recording(&self) {
        self.hint.set_visible(false);
        self.terminal.set_opacity(1.0);
        self.frame.add_css_class("recording");
        self.terminal.grab_focus();
    }

    /// Recording stopped; the script is being drafted.
    pub fn show_drafting(&self) {
        self.frame.remove_css_class("recording");
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
    argv.push("--quiet".into());
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
