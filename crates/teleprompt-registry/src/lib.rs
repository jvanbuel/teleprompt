//! Every scene plugin and voice this build of teleprompt ships, registered
//! in one place (docs/design.md#crates): the one crate that knows them by
//! name. The command line builds the [`Registry`] here once and hands it
//! to each project; nothing below this crate names a plugin or a voice.

use std::sync::OnceLock;

pub use teleprompt::registry::{Recording, Registry, Voice};

mod needs;
mod scenes;
mod voices;

/// What this build has: its scene plugins, built in then installed as
/// programs of their own, found once; and its voices. A plugin program
/// starts only when first asked something.
pub fn registry() -> Registry {
    static SCENES: OnceLock<teleprompt_plugin::ScenePlugins> = OnceLock::new();
    static VOICES: OnceLock<Vec<Voice>> = OnceLock::new();
    Registry {
        scenes: SCENES.get_or_init(scenes::all),
        shipped_scenes: scenes::SHIPPED,
        voices: VOICES.get_or_init(voices::shipped),
        own_server: teleprompt_voices::openai::endpoint,
    }
}
