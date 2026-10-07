//! Desktop scenes on macOS: the app on the Mac's own screen.
//!
//! JavaScript for Automation (`osascript -l JavaScript`) brings the app
//! forward, fits its window to the frame and drives it; ffmpeg records the
//! screen with wall-clock timestamps, cropped to the window, which is what
//! [`crate::desktop::run`] cuts shots by. The language and its timing are the
//! crate's, shared with [`crate::desktop::x11`].
//!
//! It needs, for whatever runs `teleprompt` (a terminal, the app):
//! Accessibility, to press keys and move the pointer, and Screen Recording,
//! for ffmpeg to see the screen.

mod capture;
mod jxa;

pub use capture::MacosRender;

/// The plugin's name, and the scene a block names to use it.
pub const PLUGIN_NAME: &str = "macos";

/// The macos plugin's compiler.
pub const SCENE: crate::desktop::DesktopScene = crate::desktop::DesktopScene { kind: PLUGIN_NAME };

/// The plugin, to register.
pub fn plugin() -> teleprompt_scene::ScenePlugin {
    teleprompt_scene::ScenePlugin::new(SCENE, MacosRender::default())
}
