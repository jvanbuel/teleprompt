//! Motion graphics, drawn by an existing Remotion project.
//!
//! [`scene`] reads a block — which composition, with which props — and
//! [`capture`] asks the project's own Remotion install to render it at the
//! length the narration gives it. The reverse of
//! `docs/integrations/remotion.md`, where Remotion owns the whole video.

pub mod capture;
pub mod scene;

pub use capture::RemotionRender;
pub use scene::RemotionScene;
