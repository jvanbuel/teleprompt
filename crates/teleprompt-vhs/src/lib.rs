//! Terminal scenes, recorded by VHS.
//!
//! [`scene`] is the compile-time half: it reads a tape, splits it into
//! shots and says how long each takes. [`capture`] runs `vhs` and records.

pub mod capture;
pub mod record;
pub mod scene;
pub mod tools;

pub use capture::VhsRender;
pub use record::VhsRecorder;
pub use scene::VhsScene;

/// The adapter, to register.
pub fn adapter() -> teleprompt_plugin::Adapter {
    teleprompt_plugin::Adapter::new(VhsScene, VhsRender::default()).recorded_with(VhsRecorder)
}
