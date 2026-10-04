//! Desktop scenes: an app's own window, driven and recorded.
//!
//! What every platform shares: [`script`], the action language a block is
//! written in; [`scene`], its compile-time half; and [`run`], which plays
//! a session's shots against a [`run::Screen`] and cuts them from a reel.
//! Each platform is its own scene plugin crate (`teleprompt-x11`,
//! `teleprompt-macos`), supplying the screen and the recording.

pub mod run;
pub mod scene;
pub mod script;

pub use scene::DesktopScene;
