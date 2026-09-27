//! The timeline strip under the glass: the lines as the voice says them,
//! and the shots on screen, in time. A shot is dragged by its body to
//! start on a word of a line, or after one; by its end, to stretch it.
//! Lines do not move: they are the voice's length (`crate::timeline`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk::cairo;
use gtk::prelude::*;
use teleprompt_gtk::timeline::{moved, stretched, Edit, ShotSpan, Timeline};

use super::fonts::FAMILY;

const HEIGHT: i32 = 104;
const PAD: f64 = 16.0;
const RULER: (f64, f64) = (4.0, 16.0);
const VOICE: (f64, f64) = (22.0, 30.0);
const PICTURE: (f64, f64) = (60.0, 30.0);
/// How near a shot's end, in pixels, grabs it to stretch.
const GRIP: f64 = 8.0;

pub struct TimelineStrip {
    pub root: gtk::Box,
    area: gtk::DrawingArea,
    state: Rc<RefCell<State>>,
}

#[derive(Default)]
struct State {
    timeline: Option<Timeline>,
    /// Each line's text, by id: its label, and its words for a cue.
    texts: HashMap<String, String>,
    /// The line the prompter is on.
    current: Option<String>,
    drag: Option<Dragging>,
    on_drop: Option<Rc<dyn Fn(Edit, String)>>,
}

#[derive(Clone, Copy)]
struct Dragging {
    shot: usize,
    stretch: bool,
    dx: f64,
}

impl TimelineStrip {
    pub fn new() -> Self {
        let area = gtk::DrawingArea::builder()
            .content_height(HEIGHT)
            .hexpand(true)
            .build();
        let root = gtk::Box::builder().css_classes(["timeline"]).build();
        root.append(&area);
        let state: Rc<RefCell<State>> = Rc::default();
        let strip = Self { root, area, state };
        strip.draw();
        strip.interact();
        strip
    }

    /// Shows `timeline`, labelling its lines with `texts`.
    pub fn set(&self, timeline: Timeline, texts: HashMap<String, String>) {
        let mut state = self.state.borrow_mut();
        state.timeline = Some(timeline);
        state.texts = texts;
        state.drag = None;
        drop(state);
        self.area.queue_draw();
    }

    /// Marks the line the prompter is on.
    pub fn set_current(&self, line: Option<String>) {
        self.state.borrow_mut().current = line;
        self.area.queue_draw();
    }

    /// `on_drop` hears each drag that asks for an edit, and what it does
    /// in words.
    pub fn connect_drop(&self, on_drop: impl Fn(Edit, String) + 'static) {
        self.state.borrow_mut().on_drop = Some(Rc::new(on_drop));
    }

    fn draw(&self) {
        let state = Rc::clone(&self.state);
        self.area.set_draw_func(move |area, cr, width, _| {
            let state = state.borrow();
            let Some(t) = &state.timeline else { return };
            let scale = Scale::new(t.duration_ms, f64::from(width));
            ruler(area, cr, &scale, t.duration_ms);
            for line in &t.lines {
                let (x0, x1) = (scale.x(line.start_ms), scale.x(line.end_ms));
                let current = state.current.as_deref() == Some(line.id.as_str());
                if line.recorded {
                    cr.set_source_rgba(0.24, 0.86, 0.52, 0.22);
                } else {
                    cr.set_source_rgba(0.95, 0.96, 0.97, 0.08);
                }
                rounded(cr, x0, VOICE.0, x1 - x0, VOICE.1, 5.0);
                let _ = cr.fill_preserve();
                if current {
                    cr.set_source_rgb(1.0, 0.72, 0.0);
                    cr.set_line_width(1.5);
                } else {
                    cr.set_source_rgba(0.95, 0.96, 0.97, 0.12);
                    cr.set_line_width(1.0);
                }
                let _ = cr.stroke();
                let text = state
                    .texts
                    .get(&line.id)
                    .map_or(line.id.as_str(), String::as_str);
                label(area, cr, text, x0 + 8.0, VOICE, x1 - x0 - 12.0, 0.75);
            }
            for (i, shot) in t.shots.iter().enumerate() {
                let dragging = state.drag.filter(|d| d.shot == i);
                let (mut x0, mut x1) = (scale.x(shot.start_ms), scale.x(shot.end_ms));
                if let Some(d) = dragging {
                    if d.stretch {
                        x1 = (x1 + d.dx).max(x0 + 4.0);
                    } else {
                        x0 += d.dx;
                        x1 += d.dx;
                    }
                }
                let (r, g, b) = hue(&shot.scene);
                cr.set_source_rgba(r, g, b, if shot.timed { 0.55 } else { 0.25 });
                rounded(cr, x0, PICTURE.0, (x1 - x0).max(3.0), PICTURE.1, 5.0);
                let _ = cr.fill_preserve();
                if dragging.is_some() {
                    cr.set_source_rgb(1.0, 0.72, 0.0);
                    cr.set_line_width(1.5);
                } else {
                    cr.set_source_rgba(r, g, b, 0.9);
                    cr.set_line_width(1.0);
                }
                let _ = cr.stroke();
                label(
                    area,
                    cr,
                    shot.block(),
                    x0 + 8.0,
                    PICTURE,
                    x1 - x0 - 16.0,
                    0.9,
                );
                // The grip a timed shot is stretched by.
                if shot.timed && shot.leads() && x1 - x0 > 14.0 {
                    cr.set_source_rgba(0.95, 0.96, 0.97, 0.7);
                    rounded(cr, x1 - 5.0, PICTURE.0 + 9.0, 2.0, PICTURE.1 - 18.0, 1.0);
                    let _ = cr.fill();
                }
            }
            // Where a moved shot would start, and what that means.
            if let Some(d) = state.drag {
                let shot = &t.shots[d.shot];
                let at = if d.stretch {
                    scale.x(shot.end_ms) + d.dx
                } else {
                    scale.x(shot.start_ms) + d.dx
                };
                cr.set_source_rgb(1.0, 0.72, 0.0);
                cr.set_line_width(1.0);
                cr.move_to(at.round() + 0.5, VOICE.0 - 2.0);
                cr.line_to(at.round() + 0.5, PICTURE.0 + PICTURE.1 + 2.0);
                let _ = cr.stroke();
                if let Some(drop) = preview(t, &state.texts, d, &scale) {
                    let text = describe(&drop, &state.texts, shot.line.as_deref());
                    hint(area, cr, &text, at, f64::from(width));
                }
            }
        });
    }

    fn interact(&self) {
        let drag = gtk::GestureDrag::new();
        let (state, area) = (Rc::clone(&self.state), self.area.clone());
        drag.connect_drag_begin(move |gesture, x, y| {
            let mut state = state.borrow_mut();
            let grabbed = state.timeline.as_ref().and_then(|t| {
                let scale = Scale::new(t.duration_ms, f64::from(area.width()));
                grab(t, &scale, x, y)
            });
            // A shot taken hold of is this drag's; anything else is the
            // bar's, which moves the window.
            gesture.set_state(if grabbed.is_some() {
                gtk::EventSequenceState::Claimed
            } else {
                gtk::EventSequenceState::Denied
            });
            state.drag = grabbed;
        });
        let (state, area) = (Rc::clone(&self.state), self.area.clone());
        drag.connect_drag_update(move |_, dx, _| {
            if let Some(d) = state.borrow_mut().drag.as_mut() {
                d.dx = dx;
            }
            area.queue_draw();
        });
        let (state, area) = (Rc::clone(&self.state), self.area.clone());
        drag.connect_drag_end(move |_, dx, _| {
            let (drop, on_drop) = {
                let mut s = state.borrow_mut();
                let Some(mut d) = s.drag.take() else { return };
                d.dx = dx;
                let drop = s.timeline.as_ref().and_then(|t| {
                    let scale = Scale::new(t.duration_ms, f64::from(area.width()));
                    // A click is not a drag.
                    let edit = (dx.abs() >= 3.0).then(|| preview(t, &s.texts, d, &scale))??;
                    let said = describe(&edit, &s.texts, t.shots[d.shot].line.as_deref());
                    Some((edit, said))
                });
                (drop, s.on_drop.clone())
            };
            area.queue_draw();
            if let (Some((edit, said)), Some(on_drop)) = (drop, on_drop) {
                on_drop(edit, said);
            }
        });
        self.area.add_controller(drag);

        // The pointer says what a press would grab.
        let motion = gtk::EventControllerMotion::new();
        let (state, area) = (Rc::clone(&self.state), self.area.clone());
        motion.connect_motion(move |_, x, y| {
            let state = state.borrow();
            let cursor = state.timeline.as_ref().and_then(|t| {
                let scale = Scale::new(t.duration_ms, f64::from(area.width()));
                grab(t, &scale, x, y)
            });
            area.set_cursor_from_name(match cursor {
                Some(d) if d.stretch => Some("ew-resize"),
                Some(_) => Some("grab"),
                None => None,
            });
            area.set_tooltip_text(state.timeline.as_ref().and_then(|t| {
                let scale = Scale::new(t.duration_ms, f64::from(area.width()));
                untimed_under(t, &scale, x, y).then_some(
                    "This shot states no length of its own: it takes its line's, \
                     so it moves but does not stretch",
                )
            }));
        });
        self.area.add_controller(motion);
    }
}

/// Milliseconds to pixels, across the strip's width.
struct Scale {
    per_ms: f64,
}

impl Scale {
    fn new(duration_ms: u64, width: f64) -> Self {
        Self {
            per_ms: (width - 2.0 * PAD).max(1.0) / duration_ms.max(1) as f64,
        }
    }

    fn x(&self, ms: u64) -> f64 {
        PAD + ms as f64 * self.per_ms
    }

    fn ms(&self, x: f64) -> u64 {
        ((x - PAD) / self.per_ms).max(0.0) as u64
    }
}

/// The shot a press at (`x`, `y`) takes hold of: by its end to stretch
/// (a timed shot), else by its body to move; only a block's first shot.
fn grab(t: &Timeline, scale: &Scale, x: f64, y: f64) -> Option<Dragging> {
    if !(PICTURE.0..PICTURE.0 + PICTURE.1).contains(&y) {
        return None;
    }
    let (i, shot) =
        t.shots.iter().enumerate().find(|(_, s)| {
            s.leads() && (scale.x(s.start_ms)..=scale.x(s.end_ms) + 2.0).contains(&x)
        })?;
    let stretch = shot.timed && scale.x(shot.end_ms) - x <= GRIP;
    Some(Dragging {
        shot: i,
        stretch,
        dx: 0.0,
    })
}

fn untimed_under(t: &Timeline, scale: &Scale, x: f64, y: f64) -> bool {
    (PICTURE.0..PICTURE.0 + PICTURE.1).contains(&y)
        && t.shots
            .iter()
            .any(|s| !s.timed && (scale.x(s.start_ms)..=scale.x(s.end_ms)).contains(&x))
}

/// The edit a drag would make, were it let go now.
fn preview(
    t: &Timeline,
    texts: &HashMap<String, String>,
    d: Dragging,
    scale: &Scale,
) -> Option<Edit> {
    let shot: &ShotSpan = &t.shots[d.shot];
    if d.stretch {
        stretched(shot, scale.ms(scale.x(shot.end_ms) + d.dx))
    } else {
        let at = scale.ms(scale.x(shot.start_ms) + d.dx);
        moved(t, shot, at, |id| {
            texts.get(id).map_or(1, |s| s.split_whitespace().count())
        })
    }
}

/// What a drop will do, in the author's words; `home` is the line the
/// shot is with now.
fn describe(drop: &Edit, texts: &HashMap<String, String>, home: Option<&str>) -> String {
    let word = |line: &str, n: usize| {
        texts
            .get(line)
            .and_then(|t| t.split_whitespace().nth(n))
            .unwrap_or("")
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_string()
    };
    let first_words = |line: &str| {
        let words: Vec<&str> = texts
            .get(line)
            .map(|t| t.split_whitespace().take(3).collect())
            .unwrap_or_default();
        format!("“{}…”", words.join(" "))
    };
    match drop {
        Edit::Cue { word: 0, .. } => "Start with the line".into(),
        Edit::Cue { word: n, .. } => format!("Start on “{}”", word(home.unwrap_or(""), *n)),
        Edit::Hold { .. } => "After the line".into(),
        Edit::Move {
            after,
            word: Some(n),
            ..
        } => format!("Move to {} on “{}”", first_words(after), word(after, *n)),
        Edit::Move { after, .. } => format!("Move after {}", first_words(after)),
        Edit::Stretch { by, .. } => format!("Stretch ×{by:.2}"),
    }
}

fn ruler(area: &gtk::DrawingArea, cr: &cairo::Context, scale: &Scale, duration_ms: u64) {
    let step = [1_000, 2_000, 5_000, 10_000, 30_000, 60_000]
        .into_iter()
        .find(|s| *s as f64 * scale.per_ms >= 60.0)
        .unwrap_or(120_000);
    let mut at = 0;
    while at <= duration_ms {
        let x = scale.x(at).round() + 0.5;
        cr.set_source_rgba(0.95, 0.96, 0.97, 0.18);
        cr.set_line_width(1.0);
        cr.move_to(x, RULER.0 + RULER.1 - 4.0);
        cr.line_to(x, RULER.0 + RULER.1);
        let _ = cr.stroke();
        let secs = at / 1000;
        let text = format!("{}:{:02}", secs / 60, secs % 60);
        label(area, cr, &text, x + 4.0, RULER, 60.0, 0.4);
        at += step;
    }
}

fn label(
    area: &gtk::DrawingArea,
    cr: &cairo::Context,
    text: &str,
    x: f64,
    (top, height): (f64, f64),
    width: f64,
    alpha: f64,
) {
    if width < 12.0 {
        return;
    }
    let layout = area.create_pango_layout(Some(text));
    layout.set_font_description(Some(&gtk::pango::FontDescription::from_string(&format!(
        "{FAMILY} Medium 12px"
    ))));
    layout.set_width((width * f64::from(gtk::pango::SCALE)) as i32);
    layout.set_ellipsize(gtk::pango::EllipsizeMode::End);
    layout.set_single_paragraph_mode(true);
    let (_, h) = layout.pixel_size();
    cr.set_source_rgba(0.95, 0.96, 0.97, alpha);
    cr.move_to(x, top + (height - f64::from(h)) / 2.0);
    pangocairo::functions::show_layout(cr, &layout);
}

/// A drop's meaning, in a chip above the strip where it would land.
fn hint(area: &gtk::DrawingArea, cr: &cairo::Context, text: &str, at: f64, width: f64) {
    let layout = area.create_pango_layout(Some(text));
    layout.set_font_description(Some(&gtk::pango::FontDescription::from_string(&format!(
        "{FAMILY} Bold 12px"
    ))));
    let (w, h) = layout.pixel_size();
    let (w, h) = (f64::from(w) + 16.0, f64::from(h) + 6.0);
    let x = (at - w / 2.0).clamp(4.0, (width - w - 4.0).max(4.0));
    let y = PICTURE.0 + PICTURE.1 - h - 1.0;
    cr.set_source_rgb(1.0, 0.72, 0.0);
    rounded(cr, x, y - PICTURE.1 - 8.0, w, h, h / 2.0);
    let _ = cr.fill();
    cr.set_source_rgb(0.0, 0.0, 0.0);
    cr.move_to(x + 8.0, y - PICTURE.1 - 8.0 + 3.0);
    pangocairo::functions::show_layout(cr, &layout);
}

fn rounded(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(
        x + r,
        y + h - r,
        r,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    );
    cr.arc(
        x + r,
        y + r,
        r,
        std::f64::consts::PI,
        3.0 * std::f64::consts::FRAC_PI_2,
    );
    cr.close_path();
}

/// A scene's colour: steady for its name, muted to sit under the ink.
fn hue(scene: &str) -> (f64, f64, f64) {
    const PALETTE: [(f64, f64, f64); 6] = [
        (0.48, 0.64, 1.0),
        (0.83, 0.61, 1.0),
        (0.37, 0.83, 0.88),
        (0.95, 0.55, 0.51),
        (0.43, 0.91, 0.65),
        (0.85, 0.86, 0.88),
    ];
    let n = scene
        .bytes()
        .fold(0usize, |a, b| a.wrapping_mul(31).wrapping_add(b.into()));
    PALETTE[n % PALETTE.len()]
}
