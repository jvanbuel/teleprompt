//! Browser scenes, driven by Playwright.
//!
//! One crate per tool, not one per stage: [`scene`] is the compile-time
//! half — it reads a block and says what shots are in it — and `capture`
//! is the half that runs Playwright and records. They are split by module
//! rather than by crate because nothing has ever wanted one without the
//! other, and the property the split would be protecting is already held
//! by the traits: a `SceneCompiler` has no way to run a subprocess, so
//! `check` and `plan` stay offline whatever lives beside them.

pub mod scene;

pub use scene::PlaywrightScene;
