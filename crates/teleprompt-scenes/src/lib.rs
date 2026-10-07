//! The scene plugins teleprompt ships, a module each, written against
//! [`teleprompt_scene`] alone, as an outside plugin would be
//! (`docs/guide/scene-plugins.md`).
//!
//! - [`asciinema`]: terminal recordings, played back from casts.
//! - [`desktop`]: an app's own window, driven and recorded: `x11` and `macos`.
//! - [`media`]: images, clips and title cards, drawn with ffmpeg.
//! - [`playwright`]: browser scenes.
//! - [`remotion`]: motion graphics from an existing Remotion project.
//! - [`slidev`]: slides from an existing Slidev deck.
//! - [`vhs`]: terminal scenes, recorded by VHS.

pub mod asciinema;
pub mod desktop;
pub mod media;
pub mod playwright;
pub mod remotion;
pub mod slidev;
pub mod vhs;
