//! Images, clips and title cards, drawn with nothing but ffmpeg.
//!
//! [`scene`] reads a block of directives and [`capture`] turns each into a
//! clip. The one scene with no existing tool worth adopting, so it has a
//! handful of directives of its own.

pub mod capture;
pub mod scene;

pub use capture::MediaRender;
pub use scene::MediaScene;

/// The plugin, to register.
pub fn plugin() -> teleprompt_scene::ScenePlugin {
    teleprompt_scene::ScenePlugin::new(MediaScene, MediaRender::default())
}
