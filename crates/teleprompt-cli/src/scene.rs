//! Every scene adapter this build ships, assembled in one place.

// Rust guideline compliant 2026-07-18

use teleprompt_asciinema::AsciinemaScene;
use teleprompt_media::MediaScene;
use teleprompt_playwright::PlaywrightScene;
use teleprompt_remotion::RemotionScene;
use teleprompt_scene::SceneRegistry;
use teleprompt_slidev::SlidevScene;
use teleprompt_vhs::VhsScene;

/// The scene registry every command compiles against.
///
/// Adapters are composed here for the same reason [`crate::voice::backends_for`]
/// composes the voice backends here: an adapter crate depends on
/// `teleprompt-scene` for the `SceneCompiler` contract, so `teleprompt-scene`
/// cannot depend back on it to register it.
/// [`SceneRegistry::with_builtins`] can only ever reach the adapters living
/// inside that crate — the mock — and the rest are added at the top, where
/// the dependency arrows already point.
///
/// Adding an adapter is a line here plus a crate. Nothing in `-core`,
/// `-compile`, or `-schedule` learns its name, which is the property spec
/// §7.6 asks the contract to protect.
pub fn scenes() -> SceneRegistry {
    let mut registry = SceneRegistry::with_builtins();
    registry.register(Box::new(VhsScene));
    registry.register(Box::new(PlaywrightScene));
    registry.register(Box::new(RemotionScene));
    registry.register(Box::new(SlidevScene));
    registry.register(Box::new(AsciinemaScene));
    registry.register(Box::new(MediaScene));
    registry
}
