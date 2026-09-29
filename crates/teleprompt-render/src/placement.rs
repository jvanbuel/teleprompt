//! The plan's shots as placements that tile the output, which
//! [`crate::chunk`] then cuts into chunks.

use crate::{Picture, RenderPlan};
use teleprompt_core::config::TransitionKind;
use teleprompt_core::SpanMs;

/// A slate's colour, and the padding around a clip smaller than the frame.
pub(crate) const BACKGROUND: &str = "0x0b0d10";

/// A shot plus the gaps around it. Shots do not tile the timeline (most
/// lines have no action under them), so gaps and holds are folded into a
/// neighbouring placement as frozen frames rather than cutting to blank.
pub(crate) struct Placement {
    /// Lead-in included.
    pub start_ms: u64,
    /// Frozen first frame; only the opening placement has one.
    pub lead_in_ms: u64,
    /// Lead-in, the shot, and any gap after it held on the last frame.
    pub duration_ms: u64,
    pub picture: Picture,
    /// Kind and overlap of the join with the placement before, the overlap
    /// read off the shots' offsets (see [`crate::Shot::transition`]).
    pub blend: Option<(TransitionKind, u64)>,
}

/// Covers `[0, plan.duration_ms)` exactly.
pub(crate) fn placements(plan: &RenderPlan) -> Vec<Placement> {
    let mut out: Vec<Placement> = Vec::new();
    let mut cursor = 0u64;
    let mut pending_lead = 0u64;

    let total_ms = plan.duration_ms.ms();
    for (i, shot) in plan.shots.iter().enumerate() {
        let (start_ms, duration_ms) = (shot.start_ms.ms(), shot.duration_ms.ms());
        // An overlap is a granted transition; its kind is published on the
        // shot being left.
        let overlap = cursor.saturating_sub(start_ms);
        let blend = match plan.shots[..i]
            .iter()
            .rev()
            .find(|b| b.duration_ms > SpanMs::ZERO)
        {
            Some(previous) if overlap > 0 => Some((previous.transition.kind.clone(), overlap)),
            _ => None,
        };

        // A gap is held on the previous placement's last frame, or this
        // one's first. Settled before the shot is examined, so dropping the
        // shot below does not drop the time.
        if overlap == 0 && start_ms > cursor {
            let gap = start_ms - cursor;
            match out.last_mut() {
                Some(previous) => previous.duration_ms += gap,
                None => pending_lead += gap,
            }
            cursor = start_ms;
        }

        // Dropped, not drawn: `-t 0` on a `color` source means no limit.
        if duration_ms == 0 {
            continue;
        }

        // A pause extends whatever is already on screen.
        if shot.picture == Picture::Hold {
            match out.last_mut() {
                Some(previous) => previous.duration_ms += duration_ms,
                None => pending_lead += duration_ms,
            }
            cursor = start_ms + duration_ms;
            continue;
        }

        let lead_in_ms = std::mem::take(&mut pending_lead);
        out.push(Placement {
            start_ms: start_ms - lead_in_ms,
            lead_in_ms,
            duration_ms: duration_ms + lead_in_ms,
            picture: shot.picture.clone(),
            blend,
        });
        cursor = start_ms + duration_ms;
    }

    match out.last_mut() {
        // The tail is held on the last frame too.
        Some(last) if total_ms > cursor => {
            last.duration_ms += total_ms - cursor;
        }
        // No shot drew anything, so there is no frame to hold.
        None if total_ms > 0 => out.push(Placement {
            start_ms: 0,
            lead_in_ms: 0,
            duration_ms: total_ms,
            picture: Picture::Slate,
            blend: None,
        }),
        _ => {}
    }
    out
}

/// An unknown kind fades rather than failing: the scheduler has already
/// given it time, and refusing to draw it would leave a hole.
pub(crate) fn xfade_for(kind: &TransitionKind) -> &'static str {
    match kind {
        TransitionKind::Dissolve => "dissolve",
        TransitionKind::Wipe => "wiperight",
        TransitionKind::Crossfade | TransitionKind::Cut | TransitionKind::Other(_) => "fade",
    }
}
