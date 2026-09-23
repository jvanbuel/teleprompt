//! Terminal recordings, played back from asciinema casts.
//!
//! [`scene`] reads a cast and splits it at its own markers; [`capture`]
//! renders the session with `agg` and cuts it into a clip per shot. For
//! sessions that should not be run again at capture time — a deploy, a
//! migration, a cold build — where a `vhs` tape would run every command.

pub mod capture;
pub mod scene;

pub use capture::AsciinemaRender;
pub use scene::AsciinemaScene;
