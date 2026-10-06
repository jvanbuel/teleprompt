//! The scene contract: how a block of a scene plugin's own language
//! compiles into shots, offline, so `plan` needs no tool
//! (`docs/guide/scene-plugins.md`).
//!
//! This is the half of a scene plugin that plans a video. Recording it is
//! `teleprompt-plugin`'s, which re-exports this crate as its `scene`
//! module, so a scene plugin depends on that crate alone.

pub mod contract;
pub mod mock;

pub use contract::{
    is_content, select_marked, split_at_mark, validate_commands, validate_parts, BlockSource,
    BodyOrigin, CommandError, Measured, SceneCompiler, Shot, Validated,
};
pub use mock::MockScene;

/// The scene compilers a script can use, by the name a block's scene
/// gives: what the compiler reads a block's shots from.
pub trait SceneCompilers {
    /// The compiler named `name`, if there is one.
    fn compiler(&self, name: &str) -> Option<&dyn SceneCompiler>;
    /// Every name, in the order errors list them.
    fn names(&self) -> Vec<&'static str>;
}
