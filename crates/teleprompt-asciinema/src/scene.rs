//! A recording scene: an asciinema cast, split at its own markers.
//!
//! The block is a cast — usually `include=deploy.cast`, the file `asciinema
//! rec` wrote — and the marks are the cast's own marker events,
//! `[12.5, "m", ""]`, the ones `asciinema play` pauses at. Nothing is
//! re-run: what was recorded is what is shown.
//!
//! A cast states its timing in full, so every shot's length is exact once
//! the recording's `idle_time_limit` is applied, as `asciinema play`
//! applies it. Each shot is published as a cast of its own — a v2 header
//! stating its size and `duration`, then its events from zero — which is
//! what makes `estimate` and `retime` arithmetic rather than guesses.

use teleprompt_core::{BlockId, Diagnostic, Hash};
use teleprompt_scene::contract::{BlockSource, Measured, SceneCompiler, Shot, Validated};

/// One event, at an absolute time in seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub time: f64,
    pub code: String,
    pub data: String,
}

/// A cast, read: its terminal size, its length, and its events in order.
#[derive(Debug, Clone, PartialEq)]
pub struct Cast {
    pub width: u64,
    pub height: u64,
    pub duration: f64,
    pub events: Vec<Event>,
}

/// Pauses shorter than this are typing, and are never re-timed.
const TYPING: f64 = 0.25;

/// Read a cast, v2 or v3, applying its `idle_time_limit`. Errors are
/// `(0-based line, message)`, every bad line, not only the first.
pub fn parse(body: &str) -> Result<Cast, Vec<(usize, String)>> {
    let mut lines = body
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty() && !l.trim_start().starts_with('#'));
    let Some((at, first)) = lines.next() else {
        return Err(vec![(
            0,
            "the block is empty; it should be an asciinema cast".into(),
        )]);
    };
    let header: serde_json::Value = serde_json::from_str(first)
        .map_err(|e| vec![(at, format!("the first line is not a cast header: {e}"))])?;
    let version = header["version"].as_u64();
    let (width, height) = match version {
        Some(2) => (header["width"].as_u64(), header["height"].as_u64()),
        Some(3) => (
            header["term"]["cols"].as_u64(),
            header["term"]["rows"].as_u64(),
        ),
        _ => return Err(vec![(at, "only asciicast v2 and v3 are read".into())]),
    };
    let (Some(width), Some(height)) = (width, height) else {
        return Err(vec![(
            at,
            "the header does not state the terminal's size".into(),
        )]);
    };
    let idle = header["idle_time_limit"].as_f64().unwrap_or(f64::INFINITY);
    // An explicit `duration` in a v2 header is a shot this crate wrote,
    // whose tail is part of its length.
    let stated = header["duration"].as_f64();

    let mut errors = Vec::new();
    let mut events = Vec::new();
    let (mut raw, mut time) = (0.0_f64, 0.0_f64);
    for (i, line) in lines {
        let parsed: Option<(f64, String, String)> = serde_json::from_str::<serde_json::Value>(line)
            .ok()
            .and_then(|v| {
                Some((
                    v.get(0)?.as_f64()?,
                    v.get(1)?.as_str()?.to_string(),
                    v.get(2)?.as_str().unwrap_or("").to_string(),
                ))
            });
        let Some((t, code, data)) = parsed else {
            errors.push((
                i,
                "not an event: expected `[time, \"code\", \"data\"]`".into(),
            ));
            continue;
        };
        // v3 times are intervals since the previous event; v2's are
        // absolute. Either way the gap is what the idle limit caps.
        let gap = if version == Some(3) { t } else { t - raw };
        raw = if version == Some(3) { raw + t } else { t };
        time += gap.clamp(0.0, idle);
        if matches!(code.as_str(), "o" | "r" | "m") {
            events.push(Event { time, code, data });
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let last = events.last().map_or(0.0, |e| e.time);
    Ok(Cast {
        width,
        height,
        duration: stated.unwrap_or(last).max(last),
        events,
    })
}

/// A cast as teleprompt publishes a shot: v2, stating its length.
pub fn write(cast: &Cast) -> String {
    let mut out = serde_json::json!({
        "version": 2,
        "width": cast.width,
        "height": cast.height,
        "duration": round(cast.duration),
    })
    .to_string();
    for e in &cast.events {
        out.push('\n');
        out.push_str(&serde_json::json!([round(e.time), e.code, e.data]).to_string());
    }
    out.push('\n');
    out
}

fn round(t: f64) -> f64 {
    (t * 1_000_000.0).round() / 1_000_000.0
}

/// The cast split at its markers, each part rebased to start at zero and
/// sized as the terminal was when it began. Parts of no length are left
/// out: a marker at the start, or two in a row, mark nothing.
pub fn split(cast: &Cast) -> Vec<Cast> {
    labelled(cast).into_iter().map(|(_, part)| part).collect()
}

/// [`split`], with each part's label: that of the marker it begins at,
/// empty for the first.
fn labelled(cast: &Cast) -> Vec<(String, Cast)> {
    let mut bounds: Vec<f64> = vec![0.0];
    let mut labels: Vec<String> = vec![String::new()];
    for e in cast.events.iter().filter(|e| e.code == "m") {
        bounds.push(e.time);
        labels.push(e.data.clone());
    }
    bounds.push(cast.duration);

    let (mut width, mut height) = (cast.width, cast.height);
    let mut parts = Vec::new();
    for (pair, label) in bounds.windows(2).zip(labels) {
        let (from, to) = (pair[0], pair[1]);
        let (w, h) = (width, height);
        let events: Vec<Event> = cast
            .events
            .iter()
            .filter(|e| e.code != "m" && e.time >= from && (e.time < to || to >= cast.duration))
            .map(|e| {
                if e.code == "r" {
                    if let Some((c, r)) = e.data.split_once('x') {
                        width = c.parse().unwrap_or(width);
                        height = r.parse().unwrap_or(height);
                    }
                }
                Event {
                    time: e.time - from,
                    ..e.clone()
                }
            })
            .collect();
        if to - from > 0.0 {
            parts.push((
                label,
                Cast {
                    width: w,
                    height: h,
                    duration: to - from,
                    events,
                },
            ));
        }
    }
    parts
}

/// The parts of a cast a fragment names, as one cast with a marker between
/// each: `2` is the second part (the recording after its first marker),
/// `2-3` a range of them, and anything else a marker's label.
pub fn select(cast: &Cast, fragment: &str) -> Result<Cast, String> {
    let parts = labelled(cast);
    let count = parts.len();
    let numbered = |s: &str| {
        s.trim()
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=count).contains(n))
    };
    let range = match fragment.split_once('-') {
        Some((a, b)) => numbered(a)
            .zip(numbered(b))
            .filter(|(a, b)| a <= b)
            .map(|(a, b)| a..=b),
        None => numbered(fragment).map(|n| n..=n),
    };
    let chosen: Vec<&Cast> = match range {
        Some(r) => parts[r.start() - 1..*r.end()]
            .iter()
            .map(|(_, p)| p)
            .collect(),
        None => parts
            .iter()
            .filter(|(label, _)| label == fragment)
            .map(|(_, p)| p)
            .collect(),
    };
    let Some(first) = chosen.first() else {
        let labels: Vec<&str> = parts
            .iter()
            .map(|(l, _)| l.as_str())
            .filter(|l| !l.is_empty())
            .collect();
        return Err(format!(
            "`#{fragment}` names no part of this recording, which has {count} part(s){}",
            if labels.is_empty() {
                String::new()
            } else {
                format!(" and the marker labels {}", labels.join(", "))
            }
        ));
    };
    let mut out = Cast {
        events: Vec::new(),
        duration: 0.0,
        ..(*first).clone()
    };
    for (i, part) in chosen.iter().enumerate() {
        if i > 0 {
            out.events.push(Event {
                time: out.duration,
                code: "m".into(),
                data: String::new(),
            });
        }
        let at = out.duration;
        out.events.extend(part.events.iter().map(|e| Event {
            time: at + e.time,
            ..e.clone()
        }));
        out.duration += part.duration;
    }
    Ok(out)
}

/// The same cast lasting `target` seconds, by lengthening or shortening
/// its pauses in proportion — never its typing — or `None` where the
/// pauses cannot absorb the difference.
pub fn retimed(cast: &Cast, target: f64) -> Option<Cast> {
    let mut times: Vec<f64> = vec![0.0];
    times.extend(cast.events.iter().map(|e| e.time));
    times.push(cast.duration);
    let gaps: Vec<f64> = times.windows(2).map(|w| w[1] - w[0]).collect();
    let pauses: f64 = gaps.iter().filter(|g| **g >= TYPING).sum();
    let delta = target - cast.duration;
    let new_gaps: Vec<f64> = if pauses > 0.0 {
        // Shrinking a pause stops at `TYPING`, so what may be taken is
        // bounded by how far the pauses stand above it.
        let room: f64 = gaps
            .iter()
            .filter(|g| **g >= TYPING)
            .map(|g| g - TYPING)
            .sum();
        if delta < 0.0 && -delta > room {
            return None;
        }
        gaps.iter()
            .map(|g| {
                if *g < TYPING {
                    *g
                } else if delta >= 0.0 {
                    g + delta * g / pauses
                } else {
                    g + delta * (g - TYPING) / room
                }
            })
            .collect()
    } else if delta >= 0.0 {
        // Nothing but typing: the screen holds at the end.
        let mut g = gaps.clone();
        *g.last_mut()? += delta;
        g
    } else {
        return None;
    };
    let mut t = 0.0;
    let events = cast
        .events
        .iter()
        .zip(&new_gaps)
        .map(|(e, g)| {
            t += g;
            Event {
                time: t,
                ..e.clone()
            }
        })
        .collect();
    Some(Cast {
        duration: target,
        events,
        ..cast.clone()
    })
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AsciinemaScene;

impl SceneCompiler for AsciinemaScene {
    fn kind(&self) -> &'static str {
        "asciinema"
    }

    fn validate(&self, src: &BlockSource) -> Result<Validated, Vec<Diagnostic>> {
        match parse(&src.body) {
            Ok(_) => Ok(Validated::from(src)),
            Err(errors) => Err(errors
                .into_iter()
                .map(|(line, why)| src.origin.locate(Diagnostic::error(why), line, 0))
                .collect()),
        }
    }

    fn shots(&self, v: &Validated, block_id: &BlockId) -> Result<Vec<Shot>, Vec<Diagnostic>> {
        let cast = parse(&v.body).map_err(|_| Vec::new())?;
        Ok(split(&cast)
            .iter()
            .enumerate()
            .map(|(index, part)| {
                let source = write(part);
                let hash = Hash::of(source.as_bytes());
                Shot::numbered(block_id, index, source, hash)
            })
            .collect())
    }

    fn estimate(&self, shot: &Shot) -> Measured {
        parse(&shot.source).map_or(Measured::Unknown, |c| {
            Measured::Exact(teleprompt_core::time::ms_from_seconds(c.duration))
        })
    }

    fn select(&self, body: &str, fragment: &str) -> Result<String, String> {
        let cast = parse(body).map_err(|_| "the recording is not a cast".to_string())?;
        select(&cast, fragment).map(|c| write(&c))
    }

    fn retime(&self, shot: &Shot, target_ms: u64) -> Option<String> {
        let cast = parse(&shot.source).ok()?;
        retimed(&cast, target_ms as f64 / 1000.0).map(|c| write(&c))
    }
}
