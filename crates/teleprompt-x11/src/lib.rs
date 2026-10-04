//! Desktop scenes on Linux: the app on a virtual X display.
//!
//! Each session gets its own `Xvfb` at the video's size, so the recording
//! is the app and nothing else on the author's screen, and a capture runs
//! the same on a laptop, a server or in CI. `xdotool` drives the window,
//! and `ffmpeg` records the display with wall-clock timestamps, which is
//! what [`teleprompt_desktop::run`] cuts shots by. The language and its
//! timing are `teleprompt-desktop`'s.

mod capture;
pub mod tools;
mod xdo;

pub use capture::X11Render;

/// The adapter's name, and the scene a block names to use it.
pub const ADAPTER: &str = "x11";

/// The x11 adapter's compiler.
pub const SCENE: teleprompt_desktop::DesktopScene =
    teleprompt_desktop::DesktopScene { kind: ADAPTER };

/// The adapter, to register.
pub fn adapter() -> teleprompt_plugin::Adapter {
    teleprompt_plugin::Adapter::new(SCENE, X11Render::default())
}
