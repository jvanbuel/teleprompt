//! Every adapter this build ships, assembled in one place.
//!
//! Adapter crates depend on `teleprompt-scene` and `teleprompt-capture`, so
//! they are registered here rather than in either (docs/design.md#crates).

use teleprompt_asciinema::{AsciinemaRecorder, AsciinemaRender, AsciinemaScene};
use teleprompt_capture::mock::MockCapture;
use teleprompt_capture::record::Recorder;
use teleprompt_capture::{CaptureBackend, CaptureRegistry};
use teleprompt_media::{MediaRender, MediaScene};
use teleprompt_playwright::{PlaywrightRecorder, PlaywrightRender, PlaywrightScene};
use teleprompt_remotion::{RemotionRender, RemotionScene};
use teleprompt_scene::{MockScene, SceneCompiler, SceneRegistry};
use teleprompt_slidev::{SlidevRender, SlidevScene};
use teleprompt_vhs::{VhsRecorder, VhsRender, VhsScene};

/// One adapter: how its blocks compile, how its scenes are captured, and,
/// where its tool can, how an author's session is recorded; registered
/// together so they cannot be named apart.
struct Adapter {
    compiler: Box<dyn SceneCompiler>,
    capture: Box<dyn CaptureBackend>,
    recorder: Option<Box<dyn Recorder>>,
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
        recorder: None,
    }
}

impl Adapter {
    fn recorded_with(mut self, recorder: impl Recorder + 'static) -> Self {
        assert_eq!(
            self.compiler.kind(),
            recorder.adapter(),
            "an adapter's recorder must share its name"
        );
        self.recorder = Some(Box::new(recorder));
        self
    }
}

/// Every adapter, in the order `doctor` lists their capture backends.
fn adapters() -> Vec<Adapter> {
    vec![
        adapter(VhsScene, VhsRender::default()).recorded_with(VhsRecorder),
        adapter(PlaywrightScene, PlaywrightRender::default()).recorded_with(PlaywrightRecorder),
        adapter(RemotionScene, RemotionRender::default()),
        adapter(SlidevScene, SlidevRender::default()),
        adapter(AsciinemaScene, AsciinemaRender::default()).recorded_with(AsciinemaRecorder),
        adapter(MediaScene, MediaRender::default()),
        adapter(MockScene, MockCapture::default()),
    ]
}

/// The adapters that can record a session, asciinema first: it records
/// exactly, and what it shows is what was recorded.
pub fn recorders() -> Vec<Box<dyn Recorder>> {
    let mut out: Vec<Box<dyn Recorder>> =
        adapters().into_iter().filter_map(|a| a.recorder).collect();
    out.sort_by_key(|r| r.adapter() != "asciinema");
    out
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
