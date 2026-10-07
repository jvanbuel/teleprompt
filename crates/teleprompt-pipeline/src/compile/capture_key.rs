//! The capture key: what names a shot's picture (docs/design.md#capture-key).

use std::collections::BTreeMap;
use std::path::Path;

use crate::schedule::Timeline;
use teleprompt_core::{Hash, ShotId};
use teleprompt_scene::ScenePlugins;
use teleprompt_script::config::{Config, SceneConfig};

use super::{ShotSource, PAUSE_SCENE};

/// The capture recipe version, part of every capture key. Bump it whenever
/// the picture a backend draws changes for the same script: a new renderer
/// version, a change to the tape or script teleprompt writes, new window
/// chrome, or a changed default.
///
/// Neither the plugin name nor the settings cover this. The plugin names
/// the scene language, not the program that rasterises it, and settings are
/// only what the author wrote, so a changed default moves neither and stale
/// clips would be served as current.
#[doc(hidden)]
pub const CAPTURE_RECIPE: &str = "vhs-0.11-pw-1.63-v2";

/// Names each shot's picture, which is its own source chained to every shot
/// before it in its session (docs/design.md#capture-key).
///
/// Runs after [`retime_stretched_shots`](super::retime::retime_stretched_shots), so the chain is built from the
/// source that will actually be captured.
pub(super) fn chain_capture_keys(
    timeline: &mut Timeline,
    config: &Config,
    scenes: &ScenePlugins,
    shots: &BTreeMap<ShotId, ShotSource>,
) {
    let mut chains: BTreeMap<(String, String), Hash> = BTreeMap::new();
    let mut inputs: BTreeMap<String, String> = BTreeMap::new();

    for entry in &mut timeline.entries {
        let Some(action) = entry.action.as_mut() else {
            continue;
        };
        // A pause holds whatever is on screen, so it joins no chain: later
        // keys must not depend on how long it was.
        if action.scene == PAUSE_SCENE {
            action.capture_key = action.shot_hash;
            continue;
        }

        // name(n): recipe, scene plugin, scene settings, inputs, shot source.
        let implicit = SceneConfig {
            plugin: action.plugin.clone(),
            settings: BTreeMap::new(),
            root: config.root.clone(),
        };
        let scene = Some(config.scenes.get(&action.scene).unwrap_or(&implicit));
        let settings = scene
            .map(SceneConfig::settings_fingerprint)
            .unwrap_or_default();
        // Scene-wide inputs are read once per scene.
        let inputs = inputs
            .entry(action.scene.clone())
            .or_insert_with(|| match (scene, scenes.compiler(&action.plugin)) {
                (Some(scene), Some(plugin)) => fingerprint(&plugin.inputs(scene), &scene.root),
                _ => String::new(),
            })
            .clone();
        let own = match (scene, scenes.compiler(&action.plugin)) {
            (Some(scene), Some(plugin)) => shots
                .get(&action.shot)
                .map(|s| fingerprint(&plugin.shot_inputs(scene, &s.source), &scene.root))
                .unwrap_or_default(),
            _ => String::new(),
        };
        let shot_hash = action.shot_hash.to_string();
        let mut fields = vec![CAPTURE_RECIPE, &action.plugin, &settings];
        // Empty inputs are left out, so they do not change the key.
        if !inputs.is_empty() {
            fields.push(&inputs);
        }
        if !own.is_empty() {
            fields.push(&own);
        }
        fields.push(&shot_hash);
        let name = Hash::of_fields(&fields);

        // A scene plugin whose shots do not continue is keyed by name(n) alone.
        if scenes
            .compiler(&action.plugin)
            .is_some_and(|scene| !scene.continues())
        {
            action.capture_key = Hash::of_fields(&[&name.to_string()]);
            continue;
        }

        // One chain per (scene, session). The session name is not hashed:
        // two sessions that open with the same shots share clips until
        // they diverge.
        let key = (
            action.scene.clone(),
            action.session.clone().unwrap_or_default(),
        );
        let chain = match chains.get(&key) {
            None => Hash::of_fields(&[&name.to_string()]),
            Some(previous) => Hash::of_fields(&[&previous.to_string(), &name.to_string()]),
        };
        chains.insert(key, chain);
        action.capture_key = chain;
    }
}

/// A hash of the contents of `paths`, as [`SceneCompiler::inputs`]
/// describes them: directories recursively in a stable order, skipping
/// `node_modules` and dot-entries. Empty when there is nothing to read.
///
/// [`SceneCompiler::inputs`]: teleprompt_scene::SceneCompiler::inputs
fn fingerprint(paths: &[std::path::PathBuf], root: &Path) -> String {
    fn walk(path: &Path, root: &Path, out: &mut Vec<String>) {
        if path.is_dir() {
            let Ok(entries) = std::fs::read_dir(path) else {
                return;
            };
            let mut children: Vec<_> = entries.flatten().map(|e| e.path()).collect();
            children.sort();
            for child in children {
                let name = child.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name != "node_modules" && !name.starts_with('.') {
                    walk(&child, root, out);
                }
            }
        } else if let Ok(hash) = Hash::of_file(path) {
            // Named as from the project, so the key is the same anywhere.
            let name = path.strip_prefix(root).unwrap_or(path);
            out.push(format!("{}:{hash}", name.display()));
        }
    }
    let mut files = Vec::new();
    for path in paths {
        walk(path, root, &mut files);
    }
    if files.is_empty() {
        return String::new();
    }
    let fields: Vec<&str> = files.iter().map(String::as_str).collect();
    Hash::of_fields(&fields).to_string()
}
