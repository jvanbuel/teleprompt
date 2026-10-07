//! Re-timing a shot to its slot, where its scene plugin can.

use std::collections::BTreeMap;

use crate::schedule::Timeline;
use teleprompt_core::{DurationSource, Hash, PolicyKind, ShotId};
use teleprompt_scene::{Measured, ScenePlugins, Shot};

use super::ShotSource;

/// Has each scene plugin rewrite a shot whose scheduled length differs from its
/// own length, so the source captured is the one that fits its slot.
///
/// The shot's hash moves with the source, which is how slot length reaches
/// the capture key (docs/design.md#capture-key). Where a scene plugin cannot
/// re-time, the source stands and the renderer holds the last frame.
///
/// A `fit-action` shot its plugin cannot re-time any shorter is cut at its
/// slot's end, which is said: `trim-action` cuts on purpose, and a shot
/// that is too short only holds its last frame.
pub(super) fn retime_stretched_shots(
    timeline: &mut Timeline,
    shots: &mut BTreeMap<ShotId, ShotSource>,
    scenes: &ScenePlugins,
) -> Vec<String> {
    let mut warnings = Vec::new();
    for entry in &mut timeline.entries {
        let Some(action) = entry.action.as_mut() else {
            continue;
        };
        let Some(published) = shots.get_mut(&action.shot) else {
            continue;
        };
        let Some(plugin) = scenes.compiler(&action.plugin) else {
            continue;
        };

        // Only `fit-action` and `trim-action` change the number.
        let shot = Shot {
            id: action.shot.clone(),
            source: published.source.clone(),
            hash: action.shot_hash,
            index: 0,
            length: published.length,
        };
        let natural = published.length.duration_ms();
        let slot = action.duration_ms.ms();
        if natural == Some(slot) {
            continue;
        }

        if let Some(source) = plugin.retime(&shot, slot) {
            action.shot_hash = Hash::of(source.as_bytes());
            action.duration_source = DurationSource::Exact;
            published.source = source;
            published.length = Measured::Exact(slot);
        } else if let Some(natural) = natural.filter(|n| *n > slot) {
            if entry.policy == PolicyKind::FitAction {
                warnings.push(format!(
                    "shot `{}` lasts {:.1}s but its line gives it {:.1}s, and the `{}` plugin \
                     cannot shorten it: the last {:.1}s are cut",
                    action.shot,
                    natural as f64 / 1000.0,
                    slot as f64 / 1000.0,
                    action.plugin,
                    (natural - slot) as f64 / 1000.0,
                ));
            }
        }
    }
    warnings
}
