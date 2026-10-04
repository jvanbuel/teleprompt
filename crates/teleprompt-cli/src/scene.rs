//! Every scene plugin this build ships, registered in one place: each scene plugin
//! crate hands over one `ScenePlugin`, and no other crate learns its name
//! (docs/design.md#crates).

use teleprompt_plugin::capture::mock::MockCapture;
use teleprompt_plugin::capture::CaptureRegistry;
use teleprompt_plugin::protocol::{host, Kind};
use teleprompt_plugin::record::NamedRecorder;
use teleprompt_plugin::scene::{MockScene, SceneRegistry};
use teleprompt_plugin::ScenePlugin;

/// The scene plugins this build ships, in the order `setup` and errors list
/// them.
fn built_in() -> Vec<ScenePlugin> {
    vec![
        teleprompt_vhs::plugin(),
        teleprompt_playwright::plugin(),
        teleprompt_remotion::plugin(),
        teleprompt_slidev::plugin(),
        teleprompt_asciinema::plugin(),
        teleprompt_media::plugin(),
        teleprompt_x11::plugin(),
        teleprompt_macos::plugin(),
        ScenePlugin::new(MockScene, MockCapture::default()),
    ]
}

/// Every scene plugin: the built-in ones, then each plugin installed as a
/// program of its own whose name none of them has. A plugin starts only
/// when first asked something.
fn adapters() -> Vec<ScenePlugin> {
    let mut all = built_in();
    for found in host::discover() {
        if found.kind == Kind::Scene && !all.iter().any(|a| a.name() == found.name) {
            all.push(host::scene(found));
        }
    }
    all
}

/// Whether `name` is a scene plugin this build ships, which a plugin of that
/// name does not replace.
pub fn is_built_in(name: &str) -> bool {
    built_in().iter().any(|a| a.name() == name)
}

/// Every plugin's name, which a block may use as its scene.
pub fn adapter_names() -> Vec<String> {
    adapters().iter().map(|a| a.name().to_string()).collect()
}

/// What scene plugin `name` runs, capturing and then recording, each once, by
/// name; `None` when there is no such scene plugin. What `teleprompt setup
/// <plugin>` installs.
pub fn needs(name: &str) -> Option<Vec<&'static str>> {
    adapters()
        .into_iter()
        .find(|a| a.name() == name)
        .map(|a| a.needs().iter().map(|t| t.name).collect())
}

/// What every scene plugin runs, in the order they are registered.
pub fn adapter_needs() -> Vec<&'static teleprompt_plugin::tool::Tool> {
    adapters().iter().flat_map(ScenePlugin::needs).collect()
}

/// The scene plugins that can record a session, asciinema first: it records
/// exactly, and what it shows is what was recorded.
pub fn recorders() -> Vec<NamedRecorder> {
    let mut out: Vec<NamedRecorder> = adapters()
        .into_iter()
        .filter_map(ScenePlugin::into_recorder)
        .collect();
    out.sort_by_key(|r| r.adapter != "asciinema");
    out
}

/// The scene registry every command compiles against.
pub fn scenes() -> SceneRegistry {
    let mut registry = SceneRegistry::default();
    for a in adapters() {
        registry.register(a.into_scene());
    }
    registry
}

/// The capture backends a build records with.
pub fn captures() -> CaptureRegistry {
    adapters().into_iter().fold(CaptureRegistry::new(), |r, a| {
        let name = a.name();
        r.with(name, a.into_capture())
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_adapter_has_its_own_name() {
        let mut names = super::adapter_names();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 9);
    }
}
