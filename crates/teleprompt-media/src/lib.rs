//! Images, clips and title cards, drawn with nothing but ffmpeg.
//!
//! [`scene`] reads a block of directives and [`capture`] turns each into a
//! clip. The one scene with no existing tool worth adopting (spec §7.3), so
//! it has a handful of directives of its own.

pub mod capture;
pub mod scene;

pub use capture::MediaRender;
pub use scene::MediaScene;
