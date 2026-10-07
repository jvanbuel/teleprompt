//! Every scene plugin and voice this build of teleprompt ships, registered
//! in one place (docs/design.md#crates): the one module that knows them by
//! name. The command line builds the [`Registry`] here once and hands it
//! to each project; nothing below the command line names a plugin or a voice.

use std::sync::OnceLock;

pub use teleprompt_project::registry::Registry;
use teleprompt_voice::catalogue::{Shipped, VoiceCatalogue};

mod needs;
mod scenes;
mod voices;

/// What this build has: its scene plugins, built in then installed as
/// programs of their own, found once; and its voices. A plugin program
/// starts only when first asked something.
pub fn registry() -> Registry {
    static SCENES: OnceLock<teleprompt_scene::ScenePlugins> = OnceLock::new();
    static VOICES: OnceLock<Vec<Shipped>> = OnceLock::new();
    static CATALOGUE: OnceLock<VoiceCatalogue> = OnceLock::new();
    Registry {
        scenes: SCENES.get_or_init(scenes::all),
        voices: CATALOGUE.get_or_init(|| VoiceCatalogue {
            shipped: VOICES.get_or_init(voices::shipped),
            // A `backends:` key naming no shipped voice is a server of the
            // author's that speaks OpenAI's speech API.
            fallback: teleprompt_voices::openai::endpoint,
        }),
    }
}

/// What this build ships, as `setup` lists it.
pub fn shipped(registry: Registry) -> teleprompt_setup::Shipped {
    teleprompt_setup::Shipped {
        scenes: registry
            .scenes
            .iter()
            .map(|p| teleprompt_setup::Needs {
                name: p.name(),
                tools: p.needs(),
                built_in: registry.scenes.is_shipped(p.name()),
            })
            .collect(),
        voices: registry.voices.needs(),
    }
}

/// This machine, for what this build needs, with npm packages looked for
/// in the project around the working directory, if there is one.
pub fn setup_here(registry: Registry) -> teleprompt_setup::Setup {
    teleprompt_setup::Setup::detect(
        shipped(registry),
        teleprompt_project::project::root_here(registry),
    )
}
