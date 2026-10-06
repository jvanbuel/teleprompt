//! The words every teleprompt crate shares: ids, times and durations,
//! hashes, diagnostics, block and line attributes, policies and the
//! project's config. The script language is `teleprompt-script`'s.

pub mod attrs;
pub mod config;
pub mod duration;
pub mod error;
pub mod hash;
pub mod id;
pub mod policy;
pub mod progress;
pub mod said;
pub mod time;
pub mod voice;

pub use duration::{DurationMs, DurationSource};
pub use error::{Diagnostic, Diagnostics, Severity, SourceSpan};
pub use hash::Hash;
pub use id::{BlockId, ItemId, LineId, ShotId};
pub use policy::PolicyKind;
pub use progress::{Progress, Reporter, Silent};
pub use time::{SpanMs, Tempo, TimeMs};
