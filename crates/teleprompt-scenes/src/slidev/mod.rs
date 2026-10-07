//! Slides, from an existing Slidev deck.
//!
//! [`scene`] reads a block — which slide, after how many clicks — and
//! [`capture`] has the deck's own Slidev export those slides as stills.

pub mod capture;
pub mod scene;
pub mod tools;

pub use capture::SlidevRender;
pub use scene::SlidevScene;

/// The plugin, to register.
pub fn plugin() -> teleprompt_scene::ScenePlugin {
    teleprompt_scene::ScenePlugin::new(SlidevScene, SlidevRender::default())
}
