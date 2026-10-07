//! The voices a build has: the ones it ships, each with what it needs that
//! teleprompt does not, and how a `backends:` key naming none of them is
//! built. Made once by whoever composes the build, and read by everything
//! that resolves or lists a voice (docs/design.md#crates).

use std::sync::Arc;

use teleprompt_core::tool::Tool;

use crate::{Provider, VoiceBackend};

/// A voice this build ships, and what it needs that teleprompt does not:
/// a server the author runs, or a key.
pub type Shipped = (Provider, &'static Tool);

/// How a `backends:` key naming no shipped voice is built: a server of
/// the author's under the name they gave it, from its settings; or why
/// those settings do not make one. [`Provider::build`] with the name.
pub type Fallback = fn(&str, &serde_json::Value) -> Result<Arc<dyn VoiceBackend>, String>;

/// Every voice this build has, `null` aside.
#[derive(Clone, Copy)]
pub struct VoiceCatalogue {
    /// In the order errors and `setup` list them.
    pub shipped: &'static [Shipped],
    pub fallback: Fallback,
}

/// No voice but `null`, and no server of the author's: for a test.
pub static NONE: VoiceCatalogue = VoiceCatalogue {
    shipped: &[],
    fallback: |name, _| Err(format!("no voice backend here makes `{name}`")),
};

impl VoiceCatalogue {
    /// The id of each voice shipped, `null` aside.
    pub fn ids(&self) -> Vec<&'static str> {
        self.shipped.iter().map(|(p, _)| p.id).collect()
    }

    /// What each shipped voice needs.
    pub fn tools(&self) -> Vec<&'static Tool> {
        self.shipped.iter().map(|(_, needs)| *needs).collect()
    }

    /// Each shipped voice's id with what it needs.
    pub fn needs(&self) -> Vec<(&'static str, &'static Tool)> {
        self.shipped
            .iter()
            .map(|(p, needs)| (p.id, *needs))
            .collect()
    }

    /// Whether `name` is a voice this build ships, `null` included.
    pub fn is_shipped(&self, name: &str) -> bool {
        name == "null" || self.shipped.iter().any(|(p, _)| p.id == name)
    }
}
