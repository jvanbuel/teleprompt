//! The glass as the timeline, in Edit mode: each shot a ribbon under the
//! words it plays over, or a pill in the pause after its line. A ribbon is
//! dragged onto a word to start there, or into a pause to play after that
//! line; its grip, onto a word of its line, to end there. A word hovered
//! is the moment it is said, for the monitor to show.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{cairo, graphene};
use teleprompt_gtk::ribbons::{dropped_after, dropped_on, moment, ribbons, stretched_to};
use teleprompt_gtk::timeline::{Edit, Timeline};

use super::fonts::FAMILY;
use super::prompter::Layout;

/// A ribbon's thickness, and its gap below the words.
const BAR: (f64, f64) = (5.0, 3.0);
const PILL: f64 = 24.0;

pub struct RibbonLayer {
    pub area: gtk::DrawingArea,
    view: gtk::TextView,
    layout: Rc<RefCell<Layout>>,
    state: Rc<RefCell<State>>,
}

#[derive(Default)]
struct State {
    timeline: Option<Timeline>,
    /// The script's lines, as ids and texts.
    lines: Vec<(String, String)>,
    editing: bool,
    drag: Option<Dragging>,
    /// The word under the pointer, as line and word.
    hovered: Option<(usize, usize)>,
    on_drop: Option<Rc<dyn Fn(Edit, String)>>,
    on_scrub: Option<Rc<dyn Fn(Option<u64>)>>,
}

#[derive(Clone, Copy)]
struct Dragging {
    shot: usize,
    grip: bool,
    /// The pointer, in the view's coordinates.
    x: f64,
    y: f64,
    moved: bool,
}

#[derive(Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    fn contains(&self, x: f64, y: f64, slack: f64) -> bool {
        (self.x - slack..=self.x + self.w + slack).contains(&x)
            && (self.y - slack..=self.y + self.h + slack).contains(&y)
    }
}

/// A ribbon as drawn, in the view's coordinates.
struct Piece {
    shot: usize,
    scene: String,
    bars: Vec<Rect>,
    pill: Option<(Rect, String)>,
    grip: Option<Rect>,
    /// A block's first shot, which a drag moves; the others follow it.
    leads: bool,
}

/// Where a drop lands: a line, and the word on it, or `None` for the
/// pause after it.
type Onto = (usize, Option<usize>);

impl RibbonLayer {
    pub fn new(view: &gtk::TextView, layout: Rc<RefCell<Layout>>) -> Self {
        let area = gtk::DrawingArea::builder().can_target(false).build();
        let layer = Self {
            area,
            view: view.clone(),
            layout,
            state: Rc::default(),
        };
        layer.draw();
        layer.interact();
        layer
    }

    pub fn set(&self, timeline: Timeline, lines: Vec<(String, String)>) {
        let mut state = self.state.borrow_mut();
        state.timeline = Some(timeline);
        state.lines = lines;
        state.drag = None;
        drop(state);
        self.area.queue_draw();
    }

    pub fn set_editing(&self, editing: bool) {
        let mut state = self.state.borrow_mut();
        state.editing = editing;
        state.drag = None;
        let left = state.hovered.take().is_some();
        let on_scrub = state.on_scrub.clone();
        drop(state);
        if left {
            if let Some(on_scrub) = on_scrub {
                on_scrub(None);
            }
        }
        self.view.set_cursor_from_name(None);
        self.area.queue_draw();
    }

    pub fn connect_drop(&self, on_drop: impl Fn(Edit, String) + 'static) {
        self.state.borrow_mut().on_drop = Some(Rc::new(on_drop));
    }

    /// `on_scrub` hears when the word hovered is said, and `None` as the
    /// pointer leaves the words.
    pub fn connect_scrub(&self, on_scrub: impl Fn(Option<u64>) + 'static) {
        self.state.borrow_mut().on_scrub = Some(Rc::new(on_scrub));
    }

    fn draw(&self) {
        let (view, layout, state) = (self.view.clone(), self.layout.clone(), self.state.clone());
        self.area.set_draw_func(move |area, cr, _, _| {
            let state = state.borrow();
            let Some(t) = state.timeline.as_ref().filter(|_| state.editing) else {
                return;
            };
            let origin = view
                .compute_point(area, &graphene::Point::new(0.0, 0.0))
                .unwrap_or_else(|| graphene::Point::new(0.0, 0.0));
            cr.translate(f64::from(origin.x()), f64::from(origin.y()));
            let layout = layout.borrow();
            let pieces = pieces(&view, &layout, t, &state.lines);
            for p in &pieces {
                let dragged = state.drag.is_some_and(|d| d.shot == p.shot && d.moved);
                paint(&view, cr, p, if dragged { 0.3 } else { 0.85 });
            }
            if let Some(d) = state.drag.filter(|d| d.moved) {
                let onto = onto(&view, &layout, d.x, d.y);
                if let Some(target) = onto.and_then(|o| target(&view, &layout, o)) {
                    cr.set_source_rgb(1.0, 0.72, 0.0);
                    rounded(cr, target.x, target.y, target.w, target.h, 2.0);
                    let _ = cr.fill();
                }
                let said = onto
                    .and_then(|o| edit_for(t, &state.lines, d, o))
                    .map(|e| describe(&e, &texts(&state.lines), t.shots[d.shot].line.as_deref()));
                chip(&view, cr, said.as_deref().unwrap_or("Not here"), d.x, d.y);
            }
        });
    }

    fn interact(&self) {
        // Before the view's own, which selects text: a press on a ribbon is
        // this drag's, anything else the view's.
        let drag = gtk::GestureDrag::new();
        drag.set_propagation_phase(gtk::PropagationPhase::Capture);
        let (view, layout, state, area) = self.parts();
        drag.connect_drag_begin(move |gesture, x, y| {
            let grabbed = {
                let mut s = state.borrow_mut();
                let grabbed = s.timeline.as_ref().filter(|_| s.editing).and_then(|t| {
                    let pieces = pieces(&view, &layout.borrow(), t, &s.lines);
                    grab(&pieces, x, y)
                });
                s.drag = grabbed.map(|(shot, grip)| Dragging {
                    shot,
                    grip,
                    x,
                    y,
                    moved: false,
                });
                grabbed.is_some()
            };
            // Denied ends the drag at once, in drag_end, which borrows the
            // state again.
            gesture.set_state(if grabbed {
                gtk::EventSequenceState::Claimed
            } else {
                gtk::EventSequenceState::Denied
            });
            area.queue_draw();
        });
        let (_, _, state, area) = self.parts();
        drag.connect_drag_update(move |gesture, dx, dy| {
            let Some((x, y)) = gesture.start_point() else {
                return;
            };
            if let Some(d) = state.borrow_mut().drag.as_mut() {
                (d.x, d.y) = (x + dx, y + dy);
                d.moved |= dx.hypot(dy) >= 3.0;
            }
            area.queue_draw();
        });
        let (view, layout, state, area) = self.parts();
        drag.connect_drag_end(move |_, _, _| {
            let (dropped, on_drop) = {
                let mut s = state.borrow_mut();
                let Some(d) = s.drag.take().filter(|d| d.moved) else {
                    return;
                };
                let dropped = s.timeline.as_ref().and_then(|t| {
                    let edit = edit_for(t, &s.lines, d, onto(&view, &layout.borrow(), d.x, d.y)?)?;
                    let said = describe(&edit, &texts(&s.lines), t.shots[d.shot].line.as_deref());
                    Some((edit, said))
                });
                (dropped, s.on_drop.clone())
            };
            area.queue_draw();
            if let (Some((edit, said)), Some(on_drop)) = (dropped, on_drop) {
                on_drop(edit, said);
            }
        });
        self.view.add_controller(drag);
        self.hover();
    }

    /// The pointer says what a press would grab, and a word hovered is
    /// scrubbed to.
    fn hover(&self) {
        let motion = gtk::EventControllerMotion::new();
        let (view, layout, state, _) = self.parts();
        motion.connect_motion(move |_, x, y| {
            let (scrub, on_scrub) = {
                let mut s = state.borrow_mut();
                if !s.editing || s.drag.is_some() {
                    return;
                }
                let Some(t) = &s.timeline else { return };
                let layout = layout.borrow();
                let grabbed = grab(&pieces(&view, &layout, t, &s.lines), x, y);
                view.set_cursor_from_name(match grabbed {
                    Some((_, true)) => Some("ew-resize"),
                    Some(_) => Some("grab"),
                    None => None,
                });
                let word = match onto(&view, &layout, x, y) {
                    Some((line, Some(word))) => Some((line, word)),
                    _ => None,
                };
                if word == s.hovered {
                    return;
                }
                let scrub = word.map(|(line, w)| moment(t, &s.lines, line, w));
                drop(layout);
                s.hovered = word;
                (scrub, s.on_scrub.clone())
            };
            if let Some(on_scrub) = on_scrub {
                on_scrub(scrub);
            }
        });
        let (_, _, state, _) = self.parts();
        motion.connect_leave(move |_| {
            let on_scrub = {
                let mut s = state.borrow_mut();
                s.hovered.take().and(s.on_scrub.clone())
            };
            if let Some(on_scrub) = on_scrub {
                on_scrub(None);
            }
        });
        self.view.add_controller(motion);
        let area = self.area.clone();
        self.view
            .vadjustment()
            .expect("scrollable")
            .connect_value_changed(move |_| area.queue_draw());
    }

    fn parts(
        &self,
    ) -> (
        gtk::TextView,
        Rc<RefCell<Layout>>,
        Rc<RefCell<State>>,
        gtk::DrawingArea,
    ) {
        (
            self.view.clone(),
            self.layout.clone(),
            self.state.clone(),
            self.area.clone(),
        )
    }
}

fn texts(lines: &[(String, String)]) -> HashMap<String, String> {
    lines.iter().cloned().collect()
}

/// What a drag let go at `onto` asks: a body onto a word or a pause, a
/// grip onto a word of its own line.
fn edit_for(t: &Timeline, lines: &[(String, String)], d: Dragging, onto: Onto) -> Option<Edit> {
    let shot = &t.shots[d.shot];
    match (d.grip, onto) {
        (true, (line, Some(word))) => stretched_to(t, lines, shot, line, word),
        (true, (_, None)) => None,
        (false, (line, Some(word))) => Some(dropped_on(shot, lines, line, word)),
        (false, (line, None)) => Some(dropped_after(shot, lines, line)),
    }
}

/// The word `from..to` of the buffer, in the view's coordinates.
fn word_rect(view: &gtk::TextView, from: i32, to: i32) -> Rect {
    let buffer = view.buffer();
    let a = view.iter_location(&buffer.iter_at_offset(from));
    let b = view.iter_location(&buffer.iter_at_offset(to));
    let (x, y) = view.buffer_to_window_coords(gtk::TextWindowType::Widget, a.x(), a.y());
    let w = if b.y() == a.y() {
        b.x() - a.x()
    } else {
        a.width()
    };
    Rect {
        x: f64::from(x),
        y: f64::from(y),
        w: f64::from(w.max(1)),
        h: f64::from(a.height()),
    }
}

fn seconds(ms: u64) -> String {
    format!("{:.1}s", ms as f64 / 1000.0)
}

fn pieces(
    view: &gtk::TextView,
    layout: &Layout,
    t: &Timeline,
    lines: &[(String, String)],
) -> Vec<Piece> {
    // Each line's pause fills from its last word rightwards.
    let mut after: HashMap<usize, f64> = HashMap::new();
    let right = f64::from(view.width() - view.right_margin());
    ribbons(t, lines)
        .into_iter()
        .filter_map(|r| {
            let words = layout.words.get(r.line)?;
            let shot = &t.shots[r.shot];
            let mut bars: Vec<Rect> = Vec::new();
            for &(from, to) in words.get(r.words.clone())? {
                let w = word_rect(view, from, to);
                match bars.last_mut() {
                    Some(bar) if bar.y == w.y => bar.w = w.x + w.w - bar.x,
                    _ => bars.push(w),
                }
            }
            for bar in &mut bars {
                (bar.y, bar.h) = (bar.y + bar.h + BAR.1, BAR.0);
            }
            let pill = (r.held() || r.past_ms > 0).then(|| {
                let &(from, to) = words.last()?;
                let last = word_rect(view, from, to);
                let label = if r.held() {
                    format!("{} · {}", shot.block(), seconds(r.past_ms))
                } else {
                    format!("+{}", seconds(r.past_ms))
                };
                let w = f64::from(text(view, &label).pixel_size().0) + 20.0;
                let mut x = *after.entry(r.line).or_insert(last.x + last.w + 14.0);
                let mut y = last.y + (last.h - PILL) / 2.0;
                if x + w > right {
                    x = f64::from(view.left_margin());
                    y = last.y + last.h + BAR.1;
                }
                after.insert(r.line, x + w + 8.0);
                Some((Rect { x, y, w, h: PILL }, label))
            });
            let pill = pill.flatten();
            let end = pill.as_ref().map(|(p, _)| *p).or(bars.last().copied());
            let grip = end.filter(|_| shot.timed).map(|e| Rect {
                x: e.x + e.w - 3.0,
                y: e.y + e.h / 2.0 - 8.0,
                w: 6.0,
                h: 16.0,
            });
            Some(Piece {
                shot: r.shot,
                scene: shot.scene.clone(),
                bars,
                pill,
                grip,
                leads: shot.leads(),
            })
        })
        .collect()
}

/// The shot a press at (`x`, `y`) takes hold of, and whether by its grip.
fn grab(pieces: &[Piece], x: f64, y: f64) -> Option<(usize, bool)> {
    let leading = || pieces.iter().filter(|p| p.leads);
    if let Some(p) = leading().find(|p| p.grip.is_some_and(|g| g.contains(x, y, 4.0))) {
        return Some((p.shot, true));
    }
    leading()
        .find(|p| {
            p.pill.as_ref().is_some_and(|(r, _)| r.contains(x, y, 2.0))
                || p.bars.iter().any(|b| b.contains(x, y, 5.0))
        })
        .map(|p| (p.shot, false))
}

/// The line and word at (`x`, `y`); past a line's last word, or below
/// it, the pause after it.
fn onto(view: &gtk::TextView, layout: &Layout, x: f64, y: f64) -> Option<Onto> {
    let (bx, by) = view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
    let offset = view.iter_at_location(bx, by)?.offset();
    let line = layout.line_at(offset)?;
    let words = layout.words.get(line)?;
    let &(from, to) = words.last()?;
    let last = word_rect(view, from, to);
    if offset > to || y > last.y + last.h + BAR.1 + BAR.0 {
        return Some((line, None));
    }
    Some((line, words.iter().position(|&(_, end)| offset <= end)))
}

/// Where a drop at `onto` shows: under the word, or a caret after the line.
fn target(view: &gtk::TextView, layout: &Layout, (line, word): Onto) -> Option<Rect> {
    let words = layout.words.get(line)?;
    Some(match word {
        Some(w) => {
            let &(from, to) = words.get(w)?;
            let r = word_rect(view, from, to);
            Rect {
                y: r.y + r.h + BAR.1,
                h: BAR.0,
                ..r
            }
        }
        None => {
            let &(from, to) = words.last()?;
            let r = word_rect(view, from, to);
            Rect {
                x: r.x + r.w + 6.0,
                y: r.y + 6.0,
                w: 4.0,
                h: r.h - 12.0,
            }
        }
    })
}

fn text(view: &gtk::TextView, label: &str) -> gtk::pango::Layout {
    let layout = view.create_pango_layout(Some(label));
    layout.set_font_description(Some(&gtk::pango::FontDescription::from_string(&format!(
        "{FAMILY} Bold 12px"
    ))));
    layout
}

fn paint(view: &gtk::TextView, cr: &cairo::Context, p: &Piece, alpha: f64) {
    let (r, g, b) = hue(&p.scene);
    for bar in &p.bars {
        cr.set_source_rgba(r, g, b, alpha);
        rounded(cr, bar.x, bar.y, bar.w, bar.h, bar.h / 2.0);
        let _ = cr.fill();
    }
    if let Some((pill, label)) = &p.pill {
        cr.set_source_rgba(r, g, b, alpha * 0.3);
        rounded(cr, pill.x, pill.y, pill.w, pill.h, pill.h / 2.0);
        let _ = cr.fill_preserve();
        cr.set_source_rgba(r, g, b, alpha);
        cr.set_line_width(1.5);
        let _ = cr.stroke();
        let layout = text(view, label);
        let h = f64::from(layout.pixel_size().1);
        cr.set_source_rgba(0.95, 0.96, 0.97, alpha);
        cr.move_to(pill.x + 10.0, pill.y + (pill.h - h) / 2.0);
        pangocairo::functions::show_layout(cr, &layout);
    }
    if let (Some(grip), true) = (p.grip, p.leads) {
        cr.set_source_rgba(0.95, 0.96, 0.97, alpha);
        rounded(cr, grip.x, grip.y, grip.w, grip.h, 3.0);
        let _ = cr.fill();
    }
}

/// A drop's meaning, in a chip above the pointer.
fn chip(view: &gtk::TextView, cr: &cairo::Context, said: &str, x: f64, y: f64) {
    let layout = text(view, said);
    let (w, h) = layout.pixel_size();
    let (w, h) = (f64::from(w) + 16.0, f64::from(h) + 8.0);
    let (x, y) = (x - w / 2.0, y - h - 18.0);
    cr.set_source_rgb(1.0, 0.72, 0.0);
    rounded(cr, x, y, w, h, h / 2.0);
    let _ = cr.fill();
    cr.set_source_rgb(0.0, 0.0, 0.0);
    cr.move_to(x + 8.0, y + 4.0);
    pangocairo::functions::show_layout(cr, &layout);
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
        Edit::Reword { .. } => "Reworded".into(),
        Edit::Instruct { text: Some(t), .. } => format!("Said {t}"),
        Edit::Instruct { .. } => "Said as the voice would".into(),
    }
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
