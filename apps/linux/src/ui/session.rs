//! Session mode: a terminal that `teleprompt record` runs in, drafting a
//! script from what is done and said while it records, with the tool the
//! author picks: one recording the terminal, or one opening a browser.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk::prelude::*;
use teleprompt_gtk::tools::Tool;
use vte4::prelude::*;

/// The session page: the terminal behind glass, and what to do before it
/// records.
pub struct SessionPage {
    pub root: gtk::Overlay,
    pub terminal: vte4::Terminal,
    frame: gtk::Box,
    hint: gtk::Box,
    press: gtk::Box,
    hint_title: gtk::Label,
    hint_body: gtk::Label,
    /// The tool picker: one toggle per tool, under the hint.
    tools: gtk::Box,
    chosen: Rc<RefCell<Option<String>>>,
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
        let tools = gtk::Box::builder()
            .css_classes(["linked", "session-tools"])
            .halign(gtk::Align::Center)
            .margin_top(12)
            .build();
        hint.append(&press);
        hint.append(&hint_body);
        hint.append(&tools);
        let root = gtk::Overlay::builder().child(&frame).build();
        root.add_overlay(&hint);
        Self {
            root,
            terminal,
            frame,
            hint,
            press,
            hint_title,
            hint_body,
            tools,
            chosen: Rc::default(),
        }
    }

    /// Offers `tools`, with `chosen` selected; a tool that cannot record
    /// here is shown, greyed, with why. `on_choose` hears a new choice.
    pub fn set_tools(
        &self,
        tools: &[Tool],
        chosen: Option<&str>,
        on_choose: impl Fn(&str) + 'static,
    ) {
        while let Some(child) = self.tools.first_child() {
            self.tools.remove(&child);
        }
        *self.chosen.borrow_mut() = chosen.map(str::to_string);
        let on_choose = Rc::new(on_choose);
        let mut group: Option<gtk::ToggleButton> = None;
        for tool in tools {
            let button = gtk::ToggleButton::builder()
                .label(tool.label())
                .active(Some(tool.adapter.as_str()) == chosen)
                .sensitive(tool.unavailable.is_none())
                .build();
            if let Some(why) = &tool.unavailable {
                button.set_tooltip_text(Some(why));
            }
            button.set_group(group.as_ref());
            let (adapter, chosen, on_choose) = (
                tool.adapter.clone(),
                Rc::clone(&self.chosen),
                Rc::clone(&on_choose),
            );
            button.connect_toggled(move |b| {
                if b.is_active() {
                    *chosen.borrow_mut() = Some(adapter.clone());
                    on_choose(&adapter);
                }
            });
            self.tools.append(&button);
            group.get_or_insert(button);
        }
        // One tool is no choice.
        self.tools.set_visible(tools.len() > 1);
    }

    /// The tool picked, if the tools are known yet.
    pub fn chosen(&self) -> Option<String> {
        self.chosen.borrow().clone()
    }

    /// Before recording: what the record key does, and where the draft goes.
    pub fn show_idle(&self, script: &Path) {
        self.show_hint(
            "to start recording",
            &format!(
                "Work and talk as if showing someone, then press it again to \
                 stop. What you said becomes the lines of {}, with what you \
                 did recorded between them.",
                name(script)
            ),
        );
        self.tools.set_sensitive(true);
        let choices = std::iter::successors(self.tools.first_child(), |c| c.next_sibling());
        self.tools.set_visible(choices.count() > 1);
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
        self.press.set_visible(true);
        self.hint.set_visible(true);
        self.terminal.set_opacity(0.0);
        self.frame.remove_css_class("recording");
    }

    /// Recording: in the terminal, or in the tool's own window, which the
    /// page then points to.
    pub fn show_recording(&self, in_terminal: bool) {
        self.frame.add_css_class("recording");
        self.tools.set_sensitive(false);
        if in_terminal {
            self.hint.set_visible(false);
            self.terminal.set_opacity(1.0);
            self.terminal.grab_focus();
        } else {
            self.press.set_visible(false);
            self.hint_body.set_label(
                "Work in the browser window that opened, and talk as you go. \
                 Close it, or press Ctrl ⇧ Space here, to stop.",
            );
            self.tools.set_visible(false);
        }
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

/// What a session runs: `teleprompt record` for `script` with the tool
/// picked, replacing it if it exists (the save dialog has asked), with
/// punctuation if there is a model.
pub fn record_argv(
    binary: &Path,
    script: &Path,
    with: Option<&str>,
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
    if let Some(tool) = with {
        argv.extend(["--with".into(), tool.to_string()]);
    }
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
