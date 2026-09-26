//! The monitor: the shot on screen at this point in the script, and the
//! rundown of every shot with what has aired.

use std::collections::BTreeMap;

use gtk::prelude::*;
use teleprompt_gtk::state::PrompterState;

#[derive(Clone)]
pub struct Monitor {
    pub root: gtk::Box,
    pub picture: gtk::Picture,
    slate: gtk::Label,
    title: gtk::Label,
    time: gtk::Label,
    progress: gtk::ProgressBar,
    rundown: gtk::ListBox,
    rows: std::rc::Rc<std::cell::RefCell<BTreeMap<String, (gtk::ListBoxRow, gtk::Label)>>>,
}

impl Monitor {
    pub fn new() -> Self {
        let (frame, picture, slate) = Self::screen();
        let title = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["shot-title"])
            .build();
        let time = gtk::Label::builder()
            .xalign(1.0)
            .css_classes(["shot-time"])
            .build();
        let caption = gtk::Box::builder().spacing(8).margin_top(12).build();
        caption.append(&title);
        caption.append(&time);
        let progress = gtk::ProgressBar::builder()
            .css_classes(["clip-progress"])
            .margin_top(8)
            .build();
        let heading = gtk::Label::builder()
            .label("Rundown")
            .xalign(0.0)
            .css_classes(["section-title"])
            .margin_top(28)
            .margin_bottom(6)
            .build();
        let rundown = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["rundown"])
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .child(&rundown)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["monitor-pane"])
            .width_request(380)
            .build();
        for child in [
            &frame.clone().upcast::<gtk::Widget>(),
            caption.upcast_ref(),
            progress.upcast_ref(),
            heading.upcast_ref(),
            scroller.upcast_ref(),
        ] {
            root.append(child);
        }
        root.set_margin_top(0);
        for w in [
            frame.upcast_ref::<gtk::Widget>(),
            caption.upcast_ref(),
            progress.upcast_ref(),
            heading.upcast_ref(),
            scroller.upcast_ref(),
        ] {
            w.set_margin_start(20);
            w.set_margin_end(20);
        }
        frame.set_margin_top(20);
        scroller.set_margin_bottom(12);
        Self {
            root,
            picture,
            slate,
            title,
            time,
            progress,
            rundown,
            rows: Default::default(),
        }
    }

    /// The monitor's screen: the clip, or a slate, in a 16:9 frame.
    fn screen() -> (gtk::Overlay, gtk::Picture, gtk::Label) {
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Contain)
            .visible(false)
            .build();
        let slate = gtk::Label::builder()
            .wrap(true)
            .justify(gtk::Justification::Center)
            .css_classes(["slate"])
            .margin_start(24)
            .margin_end(24)
            .build();
        // A transparent 16:9 image gives the monitor its shape at any width,
        // clip or not; the clip and the slate lie over it.
        let shape = gtk::gdk::MemoryTexture::new(
            16,
            9,
            gtk::gdk::MemoryFormat::R8g8b8a8,
            &gtk::glib::Bytes::from_owned(vec![0u8; 16 * 9 * 4]),
            16 * 4,
        );
        let sizer = gtk::Picture::builder()
            .paintable(&shape)
            .content_fit(gtk::ContentFit::Fill)
            .hexpand(true)
            .build();
        let screen = gtk::Overlay::builder()
            .child(&sizer)
            .css_classes(["monitor"])
            .overflow(gtk::Overflow::Hidden)
            .valign(gtk::Align::Start)
            .build();
        screen.add_overlay(&picture);
        screen.add_overlay(&slate);
        (screen, picture, slate)
    }

    /// Lists the script's shots, and where each starts.
    pub fn set_shots(&self, state: &PrompterState) {
        while let Some(row) = self.rundown.first_child() {
            self.rundown.remove(&row);
        }
        let mut rows = self.rows.borrow_mut();
        rows.clear();
        for shot in &state.script.shots {
            let name = gtk::Label::builder()
                .label(display_name(&shot.shot))
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["rundown-name"])
                .build();
            let dot = gtk::DrawingArea::builder()
                .width_request(10)
                .height_request(10)
                .valign(gtk::Align::Center)
                .build();
            let where_ = gtk::Label::builder()
                .xalign(1.0)
                .css_classes(["rundown-where"])
                .build();
            let line = gtk::Box::builder().spacing(10).build();
            line.append(&dot);
            line.append(&name);
            line.append(&where_);
            let row = gtk::ListBoxRow::builder()
                .child(&line)
                .activatable(false)
                .build();
            let captured = shot.clip.is_some();
            let row_for_dot = row.clone();
            dot.set_draw_func(move |_, cr, w, h| {
                let (cx, cy, r) = (f64::from(w) / 2.0, f64::from(h) / 2.0, 4.0);
                cr.arc(cx, cy, r, 0.0, std::f64::consts::TAU);
                if row_for_dot.has_css_class("on-screen") {
                    cr.set_source_rgb(0.95, 0.96, 0.97);
                    let _ = cr.fill();
                } else if row_for_dot.has_css_class("aired") {
                    cr.set_source_rgba(0.95, 0.96, 0.97, 0.3);
                    let _ = cr.fill();
                } else {
                    if captured {
                        cr.set_source_rgba(0.95, 0.96, 0.97, 0.55);
                    } else {
                        cr.set_source_rgb(0.95, 0.55, 0.51);
                    }
                    cr.set_line_width(1.5);
                    let _ = cr.stroke();
                }
            });
            self.rundown.append(&row);
            rows.insert(shot.shot.clone(), (row, where_));
        }
        drop(rows);
        self.update(state);
    }

    /// Marks what has aired, what is on screen, and what is to come.
    pub fn update(&self, state: &PrompterState) {
        for shot in &state.script.shots {
            let Some((row, where_)) = self.rows.borrow().get(&shot.shot).cloned() else {
                continue;
            };
            for class in ["on-screen", "aired", "not-captured"] {
                row.remove_css_class(class);
            }
            let (class, text) = if state.playing.as_deref() == Some(shot.shot.as_str()) {
                (Some("on-screen"), "on screen".to_string())
            } else if shot.clip.is_none() {
                (Some("not-captured"), "not captured".to_string())
            } else if state.started.contains(&shot.shot) {
                (Some("aired"), "played".to_string())
            } else if shot.at.word == 0 {
                (None, format!("line {}", shot.at.line + 1))
            } else {
                (
                    None,
                    format!("line {}, word {}", shot.at.line + 1, shot.at.word + 1),
                )
            };
            if let Some(class) = class {
                row.add_css_class(class);
            }
            where_.set_label(&text);
            if let Some(dot) = row.child().and_then(|c| c.first_child()) {
                dot.queue_draw();
            }
        }
    }

    /// Shows the clip playing, named.
    pub fn show_clip(&self, shot: &str, media: &gtk::MediaFile) {
        self.picture.set_paintable(Some(media));
        self.picture.set_visible(true);
        self.slate.set_visible(false);
        self.title.set_label(display_name(shot));
        self.progress.set_fraction(0.0);
        self.time.set_label("");
    }

    /// How far the clip has played.
    pub fn show_progress(&self, media: &gtk::MediaFile) {
        let (at, total) = (media.timestamp(), media.duration());
        if total > 0 {
            self.progress.set_fraction(at as f64 / total as f64);
            self.time
                .set_label(&format!("{} / {}", seconds(at), seconds(total)));
        }
    }

    /// No clip: why, or what will be here.
    pub fn show_slate(&self, text: &str, shot: Option<&str>, missing: bool) {
        self.picture.set_paintable(None::<&gtk::gdk::Paintable>);
        self.slate.set_label(text);
        if missing {
            self.slate.add_css_class("missing");
        } else {
            self.slate.remove_css_class("missing");
        }
        self.picture.set_visible(false);
        self.slate.set_visible(true);
        self.title.set_label(shot.map(display_name).unwrap_or(""));
        self.time.set_label("");
        self.progress.set_fraction(0.0);
    }
}

/// A shot's name as a person reads it: its block, without the index when
/// there is only one.
fn display_name(shot: &str) -> &str {
    shot.strip_suffix("#0").unwrap_or(shot)
}

fn seconds(microseconds: i64) -> String {
    format!("{:.1} s", microseconds as f64 / 1e6)
}
