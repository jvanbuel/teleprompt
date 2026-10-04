//! Desktop scenes on macOS: the app on the Mac's own screen.
//!
//! JavaScript for Automation (`osascript -l JavaScript`) brings the app
//! forward, fits its window to the frame and drives it; ffmpeg records the
//! screen with wall-clock timestamps, cropped to the window, which is what
//! [`teleprompt_desktop::run`] cuts shots by. The language and its timing
//! are `teleprompt-desktop`'s, as they are the Linux adapter's.
//!
//! It needs, for whatever runs `teleprompt` (a terminal, the app):
//! Accessibility, to press keys and move the pointer, and Screen Recording,
//! for ffmpeg to see the screen.

mod capture;
mod jxa;

pub use capture::MacosRender;

/// The adapter's name, and the scene a block names to use it.
pub const ADAPTER: &str = "macos";

/// The macos adapter's compiler.
pub const SCENE: teleprompt_desktop::DesktopScene =
    teleprompt_desktop::DesktopScene { kind: ADAPTER };

/// The adapter, to register.
pub fn adapter() -> teleprompt_capture::Adapter {
    teleprompt_capture::Adapter::new(SCENE, MacosRender::default())
}
