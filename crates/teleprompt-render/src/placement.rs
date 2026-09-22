//! The plan's shots as placements that tile the output.
//!
//! Shared by both renderers: the monolithic graph builds one filter chain
//! per placement, and the incremental one cuts the same placements into cacheable
//! chunks. Deriving the geometry twice would mean two videos that agree
//! until they do not.

use crate::{Picture, RenderPlan};

/// The colour a slate holds, and what a clip narrower than the frame is
/// padded with.
pub(crate) const BACKGROUND: &str = "0x0b0d10";

/// One shot's place on screen: what to draw, and for how long.
///
/// A placement is not the same thing as a shot. The plan's shots do not tile
/// the timeline — narration opens after a lead-in, and most paragraphs have
/// no action under them at all, which on a real script is the majority of
/// the running time. Those gaps are picture too. They are folded into the
/// neighbouring placement as frozen frames rather than becoming anything of
/// their own, because the alternative is a video that cuts to a blank field
/// every time somebody keeps talking.
pub(crate) struct Placement {
    /// Where this placement begins on the output timeline, lead-in included.
    pub start_ms: u64,
    /// Frozen first frame before the clip itself starts. Only the opening
    /// placement has one: there is nothing earlier to hold.
    pub lead_in_ms: u64,
    /// Total time on screen — lead-in, the shot, and any gap after it,
    /// which is held on the last frame.
    pub duration_ms: u64,
    pub picture: Picture,
    /// How this placement is joined to the one before it, and by how much they
    /// overlap. The overlap is the transition the scheduler granted, read
    /// off the shots' own arithmetic rather than off the published
    /// duration, so there is one source of truth for when a join happens.
    pub blend: Option<(String, u64)>,
}

/// The plan's shots as placements covering `[0, plan.duration_ms)` exactly.
pub(crate) fn placements(plan: &RenderPlan) -> Vec<Placement> {
    let mut out: Vec<Placement> = Vec::new();
    let mut cursor = 0u64;
    let mut pending_lead = 0u64;

    for (i, shot) in plan.shots.iter().enumerate() {
        // A shot that starts before the one before it ended is a shot the
        // scheduler granted a transition; the kind comes from the shot
        // being left, which is the end the manifest publishes it on.
        let overlap = cursor.saturating_sub(shot.start_ms);
        let blend = match plan.shots[..i].iter().rev().find(|b| b.duration_ms > 0) {
            Some(previous) if overlap > 0 => Some((previous.transition.kind.clone(), overlap)),
            _ => None,
        };

        // Otherwise, whatever time sits between them is held: on the
        // previous placement's last frame where there is one, and on this
        // placement's first frame where there is not. This is settled before
        // the shot itself is looked at, because a shot that contributes
        // nothing still has a gap in front of it, and dropping the shot
        // must not drop the time.
        if overlap == 0 && shot.start_ms > cursor {
            let gap = shot.start_ms - cursor;
            match out.last_mut() {
                Some(previous) => previous.duration_ms += gap,
                None => pending_lead += gap,
            }
            cursor = shot.start_ms;
        }

        // A shot of no length is dropped rather than drawn. `-t 0` on a
        // `color` source does not mean "no frames" — it means no limit, and
        // ffmpeg renders until something stops it.
        if shot.duration_ms == 0 {
            continue;
        }

        // A shot with nothing of its own to show — a pause, or a scene the
        // manifest says holds — extends whatever is already on screen.
        if shot.picture == Picture::Hold {
            match out.last_mut() {
                Some(previous) => previous.duration_ms += shot.duration_ms,
                None => pending_lead += shot.duration_ms,
            }
            cursor = shot.start_ms + shot.duration_ms;
            continue;
        }

        let lead_in_ms = std::mem::take(&mut pending_lead);
        out.push(Placement {
            start_ms: shot.start_ms - lead_in_ms,
            lead_in_ms,
            duration_ms: shot.duration_ms + lead_in_ms,
            picture: shot.picture.clone(),
            blend,
        });
        cursor = shot.start_ms + shot.duration_ms;
    }

    match out.last_mut() {
        // The tail is held too: a script that ends on a sentence should end
        // on its last frame, not on a blank one.
        Some(last) if plan.duration_ms > cursor => {
            last.duration_ms += plan.duration_ms - cursor;
        }
        // Nothing was captured at all, and there is no frame to hold. This
        // is the only case that draws a slate.
        None if plan.duration_ms > 0 => out.push(Placement {
            start_ms: 0,
            lead_in_ms: 0,
            duration_ms: plan.duration_ms,
            picture: Picture::Slate,
            blend: None,
        }),
        _ => {}
    }
    out
}

/// The `xfade` transition to use for a configured transition kind.
///
/// An unknown kind fades rather than failing the render: the scheduler has
/// already granted it time on the timeline, so refusing to draw it would
/// leave a hole, and a fade is the least surprising thing to put there.
pub(crate) fn xfade_for(kind: &str) -> &'static str {
    match kind {
        "dissolve" => "dissolve",
        "wipe" => "wiperight",
        _ => "fade",
    }
}

/// Milliseconds as seconds, which is the unit ffmpeg's `-t` takes.
pub(crate) fn seconds(ms: u64) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}
