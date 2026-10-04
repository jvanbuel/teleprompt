//! Every scene plugin this build ships, registered in one place: each scene plugin
//! crate hands over one `ScenePlugin`, and no other crate learns its name
//! (docs/design.md#crates).

use std::sync::OnceLock;

use teleprompt_plugin::capture::mock::MockCapture;
use teleprompt_plugin::protocol::host;
use teleprompt_plugin::record::Recorder;
use teleprompt_plugin::scene::MockScene;
use teleprompt_plugin::{ScenePlugin, ScenePlugins};

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
/// program of its own whose name none of them has, found once. A plugin
/// program starts only when first asked something.
pub fn plugins() -> &'static ScenePlugins {
    static PLUGINS: OnceLock<ScenePlugins> = OnceLock::new();
    PLUGINS.get_or_init(|| {
        ScenePlugins::new(
            built_in()
                .into_iter()
                .chain(host::discover().into_iter().map(host::scene)),
        )
    })
}

/// Whether `name` is a scene plugin this build ships, which a plugin of that
/// name does not replace.
pub fn is_built_in(name: &str) -> bool {
    built_in().iter().any(|a| a.name() == name)
}

/// What scene plugin `name` runs, capturing and then recording, each once, by
/// name; `None` when there is no such scene plugin. What `teleprompt setup
/// <plugin>` installs.
pub fn needs(name: &str) -> Option<Vec<&'static str>> {
    plugins()
        .get(name)
        .map(|a| a.needs().iter().map(|t| t.name).collect())
}

/// What every scene plugin runs, in the order they are registered.
pub fn plugin_needs() -> Vec<&'static teleprompt_plugin::tool::Tool> {
    plugins().iter().flat_map(ScenePlugin::needs).collect()
}

/// A scene plugin's recorder, under the plugin's name, which is the scene
/// its drafts run in.
#[derive(Clone, Copy)]
pub struct Recording {
    pub plugin: &'static str,
    recorder: &'static dyn Recorder,
}

impl std::ops::Deref for Recording {
    type Target = dyn Recorder;

    fn deref(&self) -> &Self::Target {
        self.recorder
    }
}

/// The scene plugins that can record a session, asciinema first: it records
/// exactly, and what it shows is what was recorded.
pub fn recorders() -> Vec<Recording> {
    let mut out: Vec<Recording> = plugins()
        .iter()
        .filter_map(|p| {
            p.recorder().map(|recorder| Recording {
                plugin: p.name(),
                recorder,
            })
        })
        .collect();
    out.sort_by_key(|r| r.plugin != "asciinema");
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_plugin_has_its_own_name() {
        let mut names = super::plugins().names();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 9);
    }
}
