//! The scene plugins this build ships, and the installed ones found beside
//! them.

use teleprompt_plugin::capture::mock::MockCapture;
use teleprompt_plugin::protocol::host;
use teleprompt_plugin::scene::MockScene;
use teleprompt_plugin::{ScenePlugin, ScenePlugins};

/// The names of the plugins `built_in` hands over, which an installed
/// program does not replace.
pub const SHIPPED: &[&str] = &[
    "vhs",
    "playwright",
    "remotion",
    "slidev",
    "asciinema",
    "media",
    "x11",
    "macos",
    "mock",
];

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
        teleprompt_desktop::x11::plugin(),
        teleprompt_desktop::macos::plugin(),
        ScenePlugin::new(MockScene, MockCapture::default()),
    ]
}

/// Every scene plugin: the built-in ones, then each plugin installed as a
/// program of its own whose name none of them has.
pub fn all() -> ScenePlugins {
    ScenePlugins::new(
        built_in()
            .into_iter()
            .chain(host::discover().into_iter().map(host::scene)),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_plugin_has_its_own_name_and_is_listed_as_shipped() {
        let built = super::built_in();
        let mut names: Vec<&str> = built.iter().map(|p| p.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), built.len());
        let mut shipped = super::SHIPPED.to_vec();
        shipped.sort_unstable();
        assert_eq!(names, shipped);
    }
}
