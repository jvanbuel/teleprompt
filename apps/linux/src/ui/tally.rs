//! Session mode's tally bar: whether it is recording, for how long, what
//! just happened, and the keys that matter now.

use gtk::prelude::*;

/// What the tally bar says: what just happened, or what went wrong.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Status {
    pub text: String,
    pub is_error: bool,
}

impl Status {
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: false,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: true,
        }
    }
}

#[derive(Clone)]
pub struct Tally {
    pub root: gtk::Box,
    light: gtk::Box,
    timecode: gtk::Label,
    status: gtk::Label,
    keys: gtk::Box,
}

impl Tally {
    pub fn new() -> Self {
        let light = gtk::Box::builder()
            .css_classes(["tally"])
            .accessible_role(gtk::AccessibleRole::Img)
            .valign(gtk::Align::Center)
            .build();
        let timecode = gtk::Label::builder()
            .label("00:00.0")
            .css_classes(["timecode"])
            .build();
        let status = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["status"])
            .build();
        let keys = gtk::Box::builder().spacing(14).build();
        let root = gtk::Box::builder()
            .spacing(12)
            .css_classes(["tally-bar"])
            .build();
        for w in [
            light.upcast_ref::<gtk::Widget>(),
            timecode.upcast_ref(),
            status.upcast_ref(),
            keys.upcast_ref(),
        ] {
            root.append(w);
        }
        status.set_margin_start(6);
        Self {
            root,
            light,
            timecode,
            status,
            keys,
        }
    }

    pub fn set_on_air(&self, on_air: bool) {
        self.light
            .update_property(&[gtk::accessible::Property::Label(if on_air {
                "On air"
            } else {
                "Off air"
            })]);
        for w in [
            self.light.upcast_ref::<gtk::Widget>(),
            self.timecode.upcast_ref(),
        ] {
            if on_air {
                w.add_css_class("on-air");
            } else {
                w.remove_css_class("on-air");
            }
        }
    }

    pub fn set_time(&self, seconds: f64) {
        let tenths = (seconds * 10.0).floor() as u64;
        self.timecode.set_label(&format!(
            "{:02}:{:02}.{}",
            tenths / 600,
            tenths / 10 % 60,
            tenths % 10
        ));
    }

    pub fn set_status(&self, status: &Status) {
        // Said aloud once, as it changes: a take started, kept, or failed.
        if self.status.label() != status.text && !status.text.is_empty() {
            let priority = if status.is_error {
                gtk::AccessibleAnnouncementPriority::High
            } else {
                gtk::AccessibleAnnouncementPriority::Medium
            };
            self.status.announce(&status.text, priority);
        }
        self.status.set_label(&status.text);
        self.status.set_tooltip_text(Some(&status.text));
        if status.is_error {
            self.status.add_css_class("error");
        } else {
            self.status.remove_css_class("error");
        }
    }

    /// The keys worth knowing now, as keycaps and what they do.
    /// The key hints, which a narrow window leaves out.
    pub fn keys(&self) -> &gtk::Box {
        &self.keys
    }

    pub fn set_keys(&self, keys: &[(&str, &str)]) {
        while let Some(child) = self.keys.first_child() {
            self.keys.remove(&child);
        }
        for (key, action) in keys {
            let pair = gtk::Box::builder().spacing(6).build();
            pair.append(
                &gtk::Label::builder()
                    .label(*key)
                    .css_classes(["keycap"])
                    .valign(gtk::Align::Center)
                    .build(),
            );
            pair.append(
                &gtk::Label::builder()
                    .label(*action)
                    .css_classes(["key-action"])
                    .build(),
            );
            self.keys.append(&pair);
        }
    }
}
