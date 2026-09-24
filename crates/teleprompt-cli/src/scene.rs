//! Every scene adapter this build ships, assembled in one place.

use teleprompt_asciinema::AsciinemaScene;
use teleprompt_media::MediaScene;
use teleprompt_playwright::PlaywrightScene;
use teleprompt_remotion::RemotionScene;
use teleprompt_scene::SceneRegistry;
use teleprompt_slidev::SlidevScene;
use teleprompt_vhs::VhsScene;

/// The scene registry every command compiles against.
///
/// Adapter crates depend on `teleprompt-scene`, so they are registered here,
/// on top of the mock that [`SceneRegistry::with_builtins`] holds
/// (docs/design.md#crates).
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
