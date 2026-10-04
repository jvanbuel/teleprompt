//! Every adapter this build ships, registered in one place: each adapter
//! crate hands over one `Adapter`, and no other crate learns its name
//! (docs/design.md#crates).

use teleprompt_plugin::capture::mock::MockCapture;
use teleprompt_plugin::capture::CaptureRegistry;
use teleprompt_plugin::record::NamedRecorder;
use teleprompt_plugin::scene::{MockScene, SceneRegistry};
use teleprompt_plugin::Adapter;

/// Every adapter, in the order `setup` and errors list them.
fn adapters() -> Vec<Adapter> {
    vec![
        teleprompt_vhs::adapter(),
        teleprompt_playwright::adapter(),
        teleprompt_remotion::adapter(),
        teleprompt_slidev::adapter(),
        teleprompt_asciinema::adapter(),
        teleprompt_media::adapter(),
        teleprompt_x11::adapter(),
        teleprompt_macos::adapter(),
        Adapter::new(MockScene, MockCapture::default()),
    ]
}

/// Every adapter's name, which a block may use as its scene.
pub fn adapter_names() -> Vec<String> {
    adapters().iter().map(|a| a.name().to_string()).collect()
}

/// What adapter `name` runs, capturing and then recording, each once, by
/// name; `None` when there is no such adapter. What `teleprompt setup
/// <adapter>` installs.
pub fn needs(name: &str) -> Option<Vec<&'static str>> {
    adapters()
        .into_iter()
        .find(|a| a.name() == name)
        .map(|a| a.needs().iter().map(|t| t.name).collect())
}

/// What every adapter runs, in the order they are registered.
pub fn adapter_needs() -> Vec<&'static teleprompt_plugin::tool::Tool> {
    adapters().iter().flat_map(Adapter::needs).collect()
}

/// The adapters that can record a session, asciinema first: it records
/// exactly, and what it shows is what was recorded.
pub fn recorders() -> Vec<NamedRecorder> {
    let mut out: Vec<NamedRecorder> = adapters()
        .into_iter()
        .filter_map(Adapter::into_recorder)
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
