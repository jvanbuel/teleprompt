//! The tally bar: whether the take is on air, its running time, the
//! microphone's level, what just happened, and the keys that matter now.

use gtk::prelude::*;

use teleprompt_gtk::state::Status;

#[derive(Clone)]
pub struct Tally {
    pub root: gtk::Box,
    light: gtk::Box,
    timecode: gtk::Label,
    meter: gtk::LevelBar,
    status: gtk::Label,
    keys: gtk::Box,
}

impl Tally {
    pub fn new() -> Self {
        let light = gtk::Box::builder()
            .css_classes(["tally"])
            .valign(gtk::Align::Center)
            .build();
        let timecode = gtk::Label::builder()
            .label("00:00.0")
            .css_classes(["timecode"])
            .build();
        let meter = gtk::LevelBar::builder()
            .min_value(0.0)
            .max_value(1.0)
            .valign(gtk::Align::Center)
            .css_classes(["meter"])
            .visible(false)
            .build();
        // The level bar's own marks colour it by value; the meter is one colour.
        for name in [
            gtk::LEVEL_BAR_OFFSET_LOW,
            gtk::LEVEL_BAR_OFFSET_HIGH,
            gtk::LEVEL_BAR_OFFSET_FULL,
        ] {
            meter.remove_offset_value(Some(name));
        }
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
            meter.upcast_ref(),
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
            meter,
            status,
            keys,
        }
    }

    pub fn set_on_air(&self, on_air: bool) {
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

    pub fn set_level(&self, rms: f32) {
        self.meter.set_visible(true);
        // Speech sits around 0.02–0.2 RMS; a square root spreads it out.
        self.meter.set_value(f64::from(rms).sqrt().min(1.0));
    }

    pub fn set_status(&self, status: &Status) {
        self.status.set_label(&status.text);
        if status.is_error {
            self.status.add_css_class("error");
        } else {
            self.status.remove_css_class("error");
        }
    }

    /// The keys worth knowing now, as keycaps and what they do.
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
