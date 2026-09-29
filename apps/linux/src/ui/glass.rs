//! The glass: the script on black, a gutter of line numbers, a fixed
//! reading line with a cue arrow a third of the way down, and the text
//! gliding up to it as the reader goes.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{cairo, glib};
use teleprompt_gtk::state::PrompterState;
use teleprompt_gtk::voice::{self, Mark};

use super::fonts::FAMILY;
use super::mirror::Mirror;
use super::prompter::{self, Layout};
use super::ribbons::RibbonLayer;

/// Where the reading line is, as a fraction of the glass's height.
const READING_LINE: f64 = 0.36;
/// How quickly the text settles on the reading line, in seconds.
const GLIDE: f64 = 0.22;
const GUTTER: i32 = 76;

#[derive(Clone)]
pub struct Glass {
    pub root: Mirror,
    pub view: gtk::TextView,
    countdown: gtk::Label,
    gutter: gtk::DrawingArea,
    /// The shots, in Edit mode.
    pub ribbons: Rc<RibbonLayer>,
    /// The tally's red, round the glass while a take is on air.
    frame: gtk::Box,
    /// What a take did, large on the glass, with Undo when lines were kept.
    verdict: gtk::Box,
    verdict_text: gtk::Label,
    pub verdict_undo: gtk::Button,
    /// A line's words, being edited where they stand.
    editor: gtk::TextView,
    /// Over the whole glass, for a line's panel to hang from: a text view
    /// takes no children it did not place.
    anchor: gtk::Box,
    /// A line's panel: one, kept, since GTK 4.14 can crash on a popover
    /// destroyed under the pointer.
    pub panel: gtk::Popover,
    shared: Rc<Shared>,
}

#[derive(Default)]
struct Shared {
    layout: Rc<RefCell<Layout>>,
    /// Per line: recorded.
    recorded: RefCell<Vec<bool>>,
    /// Per line: reworded since its take.
    stale: RefCell<Vec<bool>>,
    /// Per line: its take says other words.
    said: RefCell<Vec<bool>>,
    /// Per line, read by a voice: where its audio comes from.
    marks: RefCell<Vec<Option<Mark>>>,
    current: Cell<usize>,
    /// The buffer offset the reading line should hold.
    target: Cell<Option<i32>>,
    gliding: Cell<bool>,
    size: Cell<f64>,
    /// Which verdict is showing, so an older one's timer leaves it.
    said_verdict: Cell<u64>,
}

impl Glass {
    pub fn new() -> Self {
        let view = gtk::TextView::builder()
            .editable(false)
            .cursor_visible(false)
            .wrap_mode(gtk::WrapMode::Word)
            .left_margin(28)
            .right_margin(72)
            .pixels_below_lines(28)
            .pixels_inside_wrap(12)
            .css_classes(["glass"])
            .focusable(true)
            .build();
        prompter::tags(&view.buffer());
        let gutter = gtk::DrawingArea::builder().width_request(GUTTER).build();
        view.set_gutter(gtk::TextWindowType::Left, Some(&gutter));
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&view)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .build();
        let reading = gtk::DrawingArea::builder().can_target(false).build();
        let countdown = gtk::Label::builder()
            .css_classes(["countdown"])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .hexpand(true)
            .vexpand(true)
            .build();
        // The count sits on a scrim, so it is never read as part of the text.
        let scrim = gtk::Box::builder()
            .css_classes(["scrim"])
            .can_target(false)
            .visible(false)
            .build();
        scrim.append(&countdown);
        let overlay = gtk::Overlay::builder()
            .child(&scrolled)
            .css_classes(["glass-frame"])
            .build();
        let shared = Rc::new(Shared {
            size: Cell::new(48.0),
            ..Shared::default()
        });
        let ribbons = Rc::new(RibbonLayer::new(&view, shared.layout.clone()));
        overlay.add_overlay(&ribbons.area);
        overlay.add_overlay(&reading);
        overlay.add_overlay(&scrim);
        let frame = gtk::Box::builder()
            .css_classes(["tally-frame"])
            .can_target(false)
            .build();
        overlay.add_overlay(&frame);
        let (verdict, verdict_text, verdict_undo) = verdict();
        overlay.add_overlay(&verdict);
        let (editor, anchor, panel) = line_tools(&overlay);
        let glass = Self {
            root: Mirror::new(&overlay),
            view,
            countdown,
            gutter,
            ribbons,
            frame,
            verdict,
            verdict_text,
            verdict_undo,
            editor,
            anchor,
            panel,
            shared,
        };
        glass.draw_gutter();
        draw_reading_line(&reading);
        // The margins let the first line and the last reach the reading
        // line; they follow the glass's height.
        let view = glass.view.clone();
        scrolled
            .vadjustment()
            .connect_page_size_notify(move |adjustment| {
                let page = adjustment.page_size();
                view.set_top_margin((page * READING_LINE) as i32);
                view.set_bottom_margin((page * (1.0 - READING_LINE)) as i32);
            });
        let gutter = glass.gutter.clone();
        scrolled
            .vadjustment()
            .connect_value_changed(move |_| gutter.queue_draw());
        // And when the text reflows, which moves lines without a scroll: a
        // resize, a new text size, the screen shown or hidden.
        let gutter = glass.gutter.clone();
        scrolled
            .vadjustment()
            .connect_changed(move |_| gutter.queue_draw());
        glass
    }

    pub fn render(&self, state: &PrompterState) {
        let layout = prompter::render(&self.view.buffer(), state);
        *self.shared.layout.borrow_mut() = layout;
        self.ribbons.area.queue_draw();
        *self.shared.recorded.borrow_mut() =
            state.script.lines.iter().map(|l| l.recorded).collect();
        *self.shared.stale.borrow_mut() = state.script.lines.iter().map(|l| l.stale).collect();
        *self.shared.said.borrow_mut() = state
            .script
            .lines
            .iter()
            .map(|l| l.said.is_some())
            .collect();
        self.update_marks(state);
        self.show_position(state);
    }

    /// The gutter's marks of where each line's audio comes from, when a
    /// voice reads the script; none when its author does.
    pub fn update_marks(&self, state: &PrompterState) {
        let voiced = state.script.voice.as_ref().is_some_and(|v| !v.listens);
        *self.shared.marks.borrow_mut() = state
            .script
            .lines
            .iter()
            .map(|l| voice::mark(l).filter(|_| voiced))
            .collect();
        self.gutter.queue_draw();
    }

    /// Where line `line` is drawn, in the view's coordinates.
    pub fn line_rect(&self, line: usize) -> Option<gtk::gdk::Rectangle> {
        let &(start, end) = self.shared.layout.borrow().lines.get(line)?;
        let buffer = self.view.buffer();
        let (top, _) = self.view.line_yrange(&buffer.iter_at_offset(start));
        let last = buffer.iter_at_offset(end);
        let (bottom, height) = self.view.line_yrange(&last);
        let (x, y) = self
            .view
            .buffer_to_window_coords(gtk::TextWindowType::Widget, 0, top);
        let width = self.view.width() - x;
        Some(gtk::gdk::Rectangle::new(
            x,
            y,
            width.max(1),
            (bottom + height - top).max(1),
        ))
    }

    /// Where line `line` will be once the glass has glided it to the
    /// reading line, in the view's coordinates.
    pub fn settled_rect(&self, line: usize) -> Option<gtk::gdk::Rectangle> {
        let rect = self.line_rect(line)?;
        let &(start, _) = self.shared.layout.borrow().lines.get(line)?;
        let buffer = self.view.buffer();
        let centre = line_centre(
            &self.view,
            &buffer.iter_at_offset(start),
            gtk::TextWindowType::Widget,
        );
        let page = self.view.vadjustment()?.page_size();
        let shift = (page * READING_LINE - centre) as i32;
        Some(gtk::gdk::Rectangle::new(
            rect.x(),
            rect.y() + shift,
            rect.width(),
            rect.height(),
        ))
    }

    /// Opens the line's panel with `content`, pointing at where line
    /// `line` settles on the reading line.
    pub fn show_panel(&self, content: &gtk::Widget, line: usize) {
        let popover = &self.panel;
        popover.set_child(Some(content));
        if let Some(rect) = self.settled_rect(line) {
            self.point_panel(rect);
        }
        popover.popup();
    }

    fn point_panel(&self, rect: gtk::gdk::Rectangle) {
        let popover = &self.panel;
        let corner = gtk::graphene::Point::new(rect.x() as f32, rect.y() as f32);
        if let Some(at) = self.view.compute_point(&self.anchor, &corner) {
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                at.x() as i32,
                at.y() as i32,
                rect.width(),
                rect.height(),
            )));
        }
    }

    /// Puts an editor with `text` over line `line`, in the glass's type,
    /// and focuses it; `None` if the line is not laid out.
    pub fn begin_edit(&self, line: usize, text: &str) -> Option<gtk::TextView> {
        let rect = self.line_rect(line)?;
        // The view's margins, less the editor's own padding.
        let (left, right) = (self.view.left_margin() - 12, self.view.right_margin());
        self.editor.set_margin_start(rect.x() + left);
        self.editor.set_margin_top((rect.y() - 10).max(0));
        self.editor
            .set_size_request((rect.width() - left - right).max(200), rect.height());
        self.editor.buffer().set_text(text);
        self.editor.set_visible(true);
        self.editor.grab_focus();
        let end = self.editor.buffer().end_iter();
        self.editor.buffer().place_cursor(&end);
        Some(self.editor.clone())
    }

    /// Takes the editor off the glass; what it held.
    pub fn end_edit(&self) -> String {
        let buffer = self.editor.buffer();
        let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        self.editor.set_visible(false);
        text.to_string()
    }

    pub fn show_position(&self, state: &PrompterState) {
        let target =
            prompter::show_position(&self.view.buffer(), &self.shared.layout.borrow(), state);
        self.shared.current.set(state.at.line);
        self.shared.target.set(target);
        self.gutter.queue_draw();
        self.glide();
    }

    /// The script line at a point in the view, for a click.
    pub fn line_at(&self, x: f64, y: f64) -> Option<usize> {
        let (bx, by) =
            self.view
                .window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        let iter = self.view.iter_at_location(bx, by)?;
        self.shared.layout.borrow().line_at(iter.offset())
    }

    pub fn set_text_size(&self, css: &gtk::CssProvider, size: f64) {
        self.shared.size.set(size);
        css.load_from_string(&format!(
            "textview.glass, textview.glass > text, textview.line-editor, \
             textview.line-editor > text {{ font-size: {size}px; }}"
        ));
        self.glide();
    }

    pub fn text_size(&self) -> f64 {
        self.shared.size.get()
    }

    /// Shows `n` large over the glass, or nothing.
    /// Frames the glass in red on air, faintly while paused.
    pub fn show_on_air(&self, on_air: bool, paused: bool) {
        for (class, on) in [("on", on_air), ("paused", paused)] {
            if on {
                self.frame.add_css_class(class);
            } else {
                self.frame.remove_css_class(class);
            }
        }
    }

    /// Says what a take did, on the glass where the reader is looking; with
    /// `undo`, offers to put it back, for longer.
    pub fn say(&self, text: &str, undo: bool) {
        let n = self.shared.said_verdict.get() + 1;
        self.shared.said_verdict.set(n);
        self.verdict_text.set_label(text);
        self.verdict_undo.set_visible(undo);
        self.verdict.set_visible(true);
        let shared = self.shared.clone();
        let verdict = self.verdict.clone();
        let wait = std::time::Duration::from_secs(if undo { 8 } else { 4 });
        glib::timeout_add_local_once(wait, move || {
            if shared.said_verdict.get() == n {
                verdict.set_visible(false);
            }
        });
    }

    /// Takes the verdict off the glass: a take began, or it was acted on.
    pub fn unsay(&self) {
        self.shared
            .said_verdict
            .set(self.shared.said_verdict.get() + 1);
        self.verdict.set_visible(false);
    }

    pub fn show_countdown(&self, n: Option<u32>) {
        let scrim = self.countdown.parent().expect("in its scrim");
        match n {
            Some(n) => {
                self.countdown.set_label(&n.to_string());
                scrim.set_visible(true);
            }
            None => scrim.set_visible(false),
        }
    }

    /// Eases the scroll so the next word's line sits on the reading line.
    fn glide(&self) {
        if self.shared.gliding.replace(true) {
            return;
        }
        let shared = self.shared.clone();
        let last: Cell<Option<i64>> = Cell::new(None);
        self.view.add_tick_callback(move |view, clock| {
            let now = clock.frame_time();
            let dt = last
                .replace(Some(now))
                .map_or(1.0 / 60.0, |t| (now - t) as f64 / 1e6);
            let Some(goal) = scroll_goal(view, shared.target.get()) else {
                shared.gliding.set(false);
                return glib::ControlFlow::Break;
            };
            let adjustment = view.vadjustment().expect("scrollable");
            let value = adjustment.value();
            // With animations off in the desktop's settings, it jumps.
            let animate = view.settings().is_gtk_enable_animations();
            if (goal - value).abs() < 0.5 || !animate {
                adjustment.set_value(goal);
                shared.gliding.set(false);
                return glib::ControlFlow::Break;
            }
            adjustment.set_value(value + (goal - value) * (1.0 - (-dt / GLIDE).exp()));
            glib::ControlFlow::Continue
        });
    }

    fn draw_gutter(&self) {
        let (view, shared) = (self.view.clone(), self.shared.clone());
        self.gutter.set_draw_func(move |area, cr, width, height| {
            // The gutter is part of the glass.
            cr.set_source_rgb(0.0, 0.0, 0.0);
            cr.rectangle(0.0, 0.0, f64::from(width), f64::from(height));
            let _ = cr.fill();
            let layout = shared.layout.borrow();
            let recorded = shared.recorded.borrow();
            let stale = shared.stale.borrow();
            let said = shared.said.borrow();
            let marks = shared.marks.borrow();
            let buffer = view.buffer();
            for (line, &(start, _)) in layout.lines.iter().enumerate() {
                let cy = line_centre(
                    &view,
                    &buffer.iter_at_offset(start),
                    gtk::TextWindowType::Left,
                );
                let current = line == shared.current.get();
                let text = area.create_pango_layout(Some(&(line + 1).to_string()));
                let font = format!("{FAMILY} {} 14px", if current { "Bold" } else { "Medium" });
                text.set_font_description(Some(&gtk::pango::FontDescription::from_string(&font)));
                let (tw, th) = text.pixel_size();
                let x = f64::from(width - 22 - tw);
                cr.set_source_rgba(0.95, 0.96, 0.97, if current { 0.9 } else { 0.55 });
                cr.move_to(x, cy - f64::from(th) / 2.0);
                pangocairo::functions::show_layout(cr, &text);
                // Beside its number, clear of the reading line's arrow.
                if recorded.get(line).copied().unwrap_or(false) {
                    tick(cr, x - 12.0, cy);
                    if said.get(line).copied().unwrap_or(false) {
                        said_otherwise(cr, x - 12.0, cy);
                    }
                } else if stale.get(line).copied().unwrap_or(false) {
                    reworded(cr, x - 12.0, cy);
                }
                match marks.get(line).copied().flatten() {
                    Some(Mark::Voiced) => waveform(cr, x - 12.0, cy, 0.7),
                    Some(Mark::Unvoiced) => waveform(cr, x - 12.0, cy, 0.25),
                    Some(Mark::Take) | None => {}
                }
            }
        });
    }
}

/// Where the adjustment should be for `offset`'s line to sit centred on the
/// reading line: measured from where the line is on screen now, so it holds
/// whatever the view's margins.
fn scroll_goal(view: &gtk::TextView, offset: Option<i32>) -> Option<f64> {
    let offset = offset?;
    let adjustment = view.vadjustment()?;
    let (page, value) = (adjustment.page_size(), adjustment.value());
    let centre = line_centre(
        view,
        &view.buffer().iter_at_offset(offset),
        gtk::TextWindowType::Widget,
    );
    let goal = value + centre - page * READING_LINE;
    Some(goal.clamp(
        adjustment.lower(),
        (adjustment.upper() - page).max(adjustment.lower()),
    ))
}

/// The vertical centre of the text on `iter`'s line, in `window`'s
/// coordinates: the glyphs' line, not the space below it.
fn line_centre(view: &gtk::TextView, iter: &gtk::TextIter, window: gtk::TextWindowType) -> f64 {
    let (y, _) = view.line_yrange(iter);
    let glyphs = view.iter_location(iter);
    let (_, wy) = view.buffer_to_window_coords(window, 0, y);
    f64::from(wy) + f64::from(glyphs.height()) / 2.0
}

/// The mark of a line reworded since its take: an amber ring, to be read
/// again.
fn reworded(cr: &cairo::Context, x: f64, y: f64) {
    cr.set_source_rgb(1.0, 0.72, 0.0);
    cr.set_line_width(2.0);
    cr.new_sub_path();
    cr.arc(x, y, 4.5, 0.0, std::f64::consts::TAU);
    let _ = cr.stroke();
}

/// Beside the tick of a take that says other words than its line: a blue
/// dot, for W to review.
fn said_otherwise(cr: &cairo::Context, x: f64, y: f64) {
    cr.set_source_rgb(0.54, 0.71, 0.97);
    cr.new_sub_path();
    cr.arc(x - 12.0, y, 3.5, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();
}

/// A line the voice reads: three bars of a waveform, bright once it has
/// made the line, faint until then.
fn waveform(cr: &cairo::Context, x: f64, y: f64, alpha: f64) {
    cr.set_source_rgba(0.95, 0.96, 0.97, alpha);
    cr.set_line_width(2.0);
    cr.set_line_cap(cairo::LineCap::Round);
    for (dx, half) in [(-4.0, 2.5), (0.0, 5.0), (4.0, 3.5)] {
        cr.move_to(x + dx, y - half);
        cr.line_to(x + dx, y + half);
    }
    let _ = cr.stroke();
}

/// The recorded mark: a small green tick.
fn tick(cr: &cairo::Context, x: f64, y: f64) {
    cr.set_source_rgb(0.24, 0.86, 0.52);
    cr.set_line_width(2.2);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.move_to(x - 5.0, y);
    cr.line_to(x - 1.5, y + 3.5);
    cr.line_to(x + 5.0, y - 4.0);
    let _ = cr.stroke();
}

/// The reading line: a cue arrow in the margin, with the glass fading out
/// above and below so the eye stays on it.
fn draw_reading_line(area: &gtk::DrawingArea) {
    area.set_draw_func(|_, cr, width, height| {
        let (w, h) = (f64::from(width), f64::from(height));
        let y = h * READING_LINE;
        let fade = |from: f64, to: f64, alpha_from: f64, alpha_to: f64| {
            let gradient = cairo::LinearGradient::new(0.0, from, 0.0, to);
            gradient.add_color_stop_rgba(0.0, 0.0, 0.0, 0.0, alpha_from);
            gradient.add_color_stop_rgba(1.0, 0.0, 0.0, 0.0, alpha_to);
            cr.rectangle(0.0, from.min(to), w, (to - from).abs());
            let _ = cr.set_source(&gradient);
            let _ = cr.fill();
        };
        fade(0.0, h * 0.2, 0.85, 0.0);
        fade(h * 0.78, h, 0.0, 0.9);
        // The arrow, in the gutter, pointing at the line.
        cr.set_source_rgb(1.0, 0.72, 0.0);
        cr.move_to(10.0, y - 8.0);
        cr.line_to(22.0, y);
        cr.line_to(10.0, y + 8.0);
        cr.close_path();
        let _ = cr.fill();
    });
}

/// What changes a line on the glass, over it: the editor for its words,
/// and its panel, with the anchor it hangs from.
fn line_tools(overlay: &gtk::Overlay) -> (gtk::TextView, gtk::Box, gtk::Popover) {
    let editor = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::Word)
        .pixels_inside_wrap(12)
        .css_classes(["line-editor"])
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .visible(false)
        .build();
    overlay.add_overlay(&editor);
    let anchor = gtk::Box::builder().can_target(false).build();
    overlay.add_overlay(&anchor);
    let panel = gtk::Popover::builder()
        .css_classes(["line-panel"])
        .position(gtk::PositionType::Bottom)
        .build();
    panel.set_parent(&anchor);
    (editor, anchor, panel)
}

/// What a take did, over the glass: its words, and Undo.
fn verdict() -> (gtk::Box, gtk::Label, gtk::Button) {
    let verdict_text = gtk::Label::builder()
        .css_classes(["verdict-text"])
        .wrap(true)
        .build();
    let verdict_undo = gtk::Button::builder()
        .label("Undo")
        .css_classes(["pill", "verdict-undo"])
        .valign(gtk::Align::Center)
        .build();
    let verdict = gtk::Box::builder()
        .css_classes(["verdict"])
        .spacing(18)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::End)
        // Above where toasts appear, so one never covers the other.
        .margin_bottom(120)
        .margin_start(16)
        .margin_end(16)
        .visible(false)
        .build();
    verdict.append(&verdict_text);
    verdict.append(&verdict_undo);
    (verdict, verdict_text, verdict_undo)
}
