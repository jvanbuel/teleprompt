//! Changing a line where it is read: its words, on the glass (F2, or
//! double-click with a voice), and, read by a voice, how to say it. Each
//! goes through `teleprompt edit`, which writes nothing that would not
//! compile, and Ctrl+Z undoes it. Anything larger belongs in the author's
//! own editor: Open in Editor.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use teleprompt_gtk::api::{Position, Source};
use teleprompt_gtk::state::Status;
use teleprompt_gtk::timeline::Edit;

use super::Window;

impl Window {
    /// Who reads, from the welcome page: saved, and a script already open
    /// is opened again for its new reader.
    pub(super) fn set_voice_reads(self: &Rc<Self>, voice: bool) {
        let mut config = self.config();
        if config.voice_reads == voice {
            return;
        }
        config.voice_reads = voice;
        self.set_config(config);
        let open = self
            .w
            .stack
            .visible_child_name()
            .is_some_and(|page| page != "welcome" && page != "session");
        if let (true, Some(script)) = (open, self.last_script()) {
            self.open(script);
        }
    }

    /// Shows who reads on the welcome page, as the settings say.
    pub(super) fn show_narrator(&self) {
        self.w
            .narrator_voice
            .set_active(self.model.borrow().config.voice_reads);
    }

    /// A line clicked, read by a voice: while it reads, it reads on from
    /// there; otherwise the line is chosen and its panel opens.
    pub(super) fn line_clicked(self: &Rc<Self>, line: usize) {
        if self.model.borrow().rewording.is_some() {
            return;
        }
        if self.is_reading() {
            return self.read_from(line, false, false);
        }
        self.model.borrow_mut().state.at = Position { line, word: 0 };
        self.w.glass.show_position(&self.model.borrow().state);
        self.line_panel(line);
    }

    /// What can be done to a line read by a voice: hear it, read on from
    /// it, tell the voice how to say it, have it said anew, or reword it.
    fn line_panel(self: &Rc<Self>, line: usize) {
        let (id, instruct, source, voice) = {
            let model = self.model.borrow();
            let script = &model.state.script;
            let Some(l) = script.lines.get(line) else {
                return;
            };
            (
                l.id.clone(),
                l.instruct.clone(),
                l.audio.as_ref().map(|a| a.source),
                script.voice.as_ref().map(|v| v.name.clone()),
            )
        };
        let popover = self.w.glass.panel.clone();
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();
        let who = match (source, &voice) {
            (Some(Source::Take), _) => "Read from your take".to_string(),
            (_, Some(name)) => format!("Read by {name}"),
            _ => String::new(),
        };
        let title = gtk::Label::builder()
            .label(format!("Line {}", line + 1))
            .xalign(0.0)
            .css_classes(["panel-title"])
            .build();
        let subtitle = gtk::Label::builder()
            .label(who)
            .xalign(0.0)
            .css_classes(["panel-subtitle"])
            .build();
        column.append(&title);
        column.append(&subtitle);

        let listen = self.panel_button("Listen", "media-playback-start-symbolic", &popover, {
            move |this| this.read_from(line, true, false)
        });
        listen.add_css_class("suggested");
        let from_here = self.panel_button("Read on from here", "", &popover, move |this| {
            this.read_from(line, false, false)
        });
        let actions = gtk::Box::builder().spacing(8).build();
        actions.append(&listen);
        actions.append(&from_here);
        column.append(&actions);

        if source == Some(Source::Voice) {
            column.append(&self.instruction(&popover, id.clone(), line, instruct));
            let again = self.panel_button("Say it again", "view-refresh-symbolic", &popover, {
                move |this| this.read_from(line, true, true)
            });
            again.set_tooltip_text(Some(
                "Have the voice make this line anew: for a voice that says a line \
                 differently each time",
            ));
            let reword = self.panel_button("Reword…", "", &popover, {
                move |this| this.begin_reword(line)
            });
            let more = gtk::Box::builder().spacing(8).build();
            more.append(&again);
            more.append(&reword);
            column.append(&more);
        } else {
            let reword = self.panel_button("Reword…", "", &popover, {
                move |this| this.begin_reword(line)
            });
            column.append(&reword);
        }
        self.w.glass.show_panel(column.upcast_ref(), line);
    }

    /// A button in a line's panel that closes it and does `run`.
    fn panel_button(
        self: &Rc<Self>,
        label: &str,
        icon: &str,
        popover: &gtk::Popover,
        run: impl Fn(&Rc<Self>) + 'static,
    ) -> gtk::Button {
        let inner = gtk::Box::builder().spacing(8).build();
        if !icon.is_empty() {
            inner.append(&gtk::Image::from_icon_name(icon));
        }
        inner.append(&gtk::Label::new(Some(label)));
        let button = gtk::Button::builder()
            .child(&inner)
            .css_classes(["pill"])
            .build();
        let (weak, popover) = (Rc::downgrade(self), popover.clone());
        button.connect_clicked(move |_| {
            popover.popdown();
            if let Some(this) = weak.upgrade() {
                run(&this);
            }
        });
        button
    }

    /// How to say the line: an entry, written to the script on Enter.
    fn instruction(
        self: &Rc<Self>,
        popover: &gtk::Popover,
        id: String,
        line: usize,
        instruct: Option<String>,
    ) -> gtk::Box {
        let entry = gtk::Entry::builder()
            .text(instruct.clone().unwrap_or_default())
            .placeholder_text("slower, amused")
            .width_chars(32)
            .build();
        entry.update_property(&[gtk::accessible::Property::Label("How to say it")]);
        let (weak, popover) = (Rc::downgrade(self), popover.clone());
        entry.connect_activate(move |entry| {
            popover.popdown();
            let Some(this) = weak.upgrade() else { return };
            let text = entry.text().trim().to_string();
            if Some(&text) == instruct.as_ref() || (text.is_empty() && instruct.is_none()) {
                return;
            }
            let done = if text.is_empty() {
                format!("Line {} said as the voice would", line + 1)
            } else {
                format!("Line {} said {text}", line + 1)
            };
            this.edit_script(
                Edit::Instruct {
                    line: id.clone(),
                    text: (!text.is_empty()).then_some(text),
                },
                done,
                "Not changed",
            );
        });
        let label = gtk::Label::builder()
            .label("How to say it")
            .xalign(0.0)
            .css_classes(["panel-subtitle"])
            .build();
        let hint = gtk::Label::builder()
            .label("For a voice that takes directions. Enter to apply.")
            .xalign(0.0)
            .css_classes(["panel-hint"])
            .build();
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .build();
        column.append(&label);
        column.append(&entry);
        column.append(&hint);
        column
    }

    /// F2: reword the line you are on.
    pub(super) fn reword_current(self: &Rc<Self>) {
        let line = self.model.borrow().state.at.line;
        self.begin_reword(line);
    }

    /// The line's words become editable where they are: Enter keeps them,
    /// Escape leaves the line as it was. Not during a take or a reading,
    /// nor on mirrored glass, which reads backwards.
    pub(super) fn begin_reword(self: &Rc<Self>, line: usize) {
        let refuse = |this: &Rc<Self>, why: &str| {
            this.model.borrow_mut().state.status = Status::info(why);
            this.show_status();
        };
        if self.is_taking() {
            return refuse(self, "Keep or discard this take first");
        }
        if self.w.glass.root.is_mirrored() {
            return refuse(
                self,
                "Mirrored text cannot be edited: M turns mirroring off",
            );
        }
        self.stop_reading(None);
        let Some(text) = self
            .model
            .borrow()
            .state
            .script
            .lines
            .get(line)
            .map(|l| l.text.clone())
        else {
            return;
        };
        let Some(editor) = self.w.glass.begin_edit(line, &text) else {
            return;
        };
        self.model.borrow_mut().rewording = Some(line);
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(this) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            match key {
                gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter => {
                    this.end_reword(true);
                    glib::Propagation::Stop
                }
                gtk::gdk::Key::Escape => {
                    this.end_reword(false);
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        editor.add_controller(keys);
        self.model.borrow_mut().state.status = Status::info(format!(
            "Rewording line {}: Enter keeps it, Esc leaves it",
            line + 1
        ));
        self.show_status();
    }

    /// Ends the rewording, writing the new words if `keep`.
    fn end_reword(self: &Rc<Self>, keep: bool) {
        let Some(line) = self.model.borrow_mut().rewording.take() else {
            return;
        };
        let words = self.w.glass.end_edit();
        self.w.glass.view.grab_focus();
        let (id, before) = {
            let model = self.model.borrow();
            let Some(l) = model.state.script.lines.get(line) else {
                return;
            };
            (l.id.clone(), l.text.clone())
        };
        // A paragraph is one line: what was typed on several is one.
        let text = words.split_whitespace().collect::<Vec<_>>().join(" ");
        if keep && !text.is_empty() && text != before {
            self.edit_script(
                Edit::Reword { line: id, text },
                format!("Reworded line {}", line + 1),
                "Not reworded",
            );
        }
        let status = if self.voice_reads() {
            self.voice_status()
        } else {
            Status::info("Press Ctrl+Shift+Space to record from here, or click a line")
        };
        self.model.borrow_mut().state.status = status;
        self.show_status();
    }

    /// The script in the author's own editor, for what the glass does not
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
}
