//! Images, clips and title cards, drawn with nothing but ffmpeg.
//!
//! [`scene`] reads a block of directives and [`capture`] turns each into a
//! clip. The one scene with no existing tool worth adopting, so it has a
//! handful of directives of its own.

pub mod capture;
pub mod scene;

pub use capture::MediaRender;
pub use scene::MediaScene;

/// The adapter, to register.
pub fn adapter() -> teleprompt_capture::Adapter {
    teleprompt_capture::Adapter::new(MediaScene, MediaRender::default())
}
