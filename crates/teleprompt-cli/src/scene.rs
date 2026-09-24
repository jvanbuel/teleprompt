//! Every adapter this build ships, assembled in one place.
//!
//! Adapter crates depend on `teleprompt-scene` and `teleprompt-capture`, so
//! they are registered here rather than in either (docs/design.md#crates).

use teleprompt_asciinema::{AsciinemaRender, AsciinemaScene};
use teleprompt_capture::mock::MockCapture;
use teleprompt_capture::{CaptureBackend, CaptureRegistry};
use teleprompt_media::{MediaRender, MediaScene};
use teleprompt_playwright::{PlaywrightRender, PlaywrightScene};
use teleprompt_remotion::{RemotionRender, RemotionScene};
use teleprompt_scene::{MockScene, SceneCompiler, SceneRegistry};
use teleprompt_slidev::{SlidevRender, SlidevScene};
use teleprompt_vhs::{VhsRender, VhsScene};

/// One adapter: how its blocks compile and how its scenes are recorded,
/// registered together so the two cannot be named apart.
struct Adapter {
    compiler: Box<dyn SceneCompiler>,
    capture: Box<dyn CaptureBackend>,
}

fn adapter(
    compiler: impl SceneCompiler + 'static,
    capture: impl CaptureBackend + 'static,
) -> Adapter {
    assert_eq!(
        compiler.kind(),
        capture.adapter(),
        "an adapter's compiler and capture backend must share its name"
    );
    Adapter {
        compiler: Box::new(compiler),
        capture: Box::new(capture),
    }
}

/// Every adapter, in the order `doctor` lists their capture backends.
fn adapters() -> Vec<Adapter> {
    vec![
        adapter(VhsScene, VhsRender::default()),
        adapter(PlaywrightScene, PlaywrightRender::default()),
        adapter(RemotionScene, RemotionRender::default()),
        adapter(SlidevScene, SlidevRender::default()),
        adapter(AsciinemaScene, AsciinemaRender::default()),
        adapter(MediaScene, MediaRender::default()),
        adapter(MockScene, MockCapture::default()),
    ]
}

/// The scene registry every command compiles against.
pub fn scenes() -> SceneRegistry {
    let mut registry = SceneRegistry::default();
    for a in adapters() {
        registry.register(a.compiler);
    }
    registry
}

/// The capture backends a build records with.
pub fn captures() -> CaptureRegistry {
    adapters()
        .into_iter()
        .fold(CaptureRegistry::new(), |r, a| r.with(a.capture))
}

#[cfg(test)]
mod tests {
    /// Building the list checks every pair's names.
    #[test]
    fn every_adapter_compiles_and_records_under_one_name() {
        assert_eq!(super::adapters().len(), 7);
    }
}
