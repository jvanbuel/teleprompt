//! An adapter, whole: how its blocks compile, how its scenes are captured
//! and, where its tool can, how an author's session is recorded, under the
//! one name a block's scene gives (`docs/design.md#crates`).

use crate::scene::SceneCompiler;

use crate::capture::CaptureBackend;
use crate::record::{NamedRecorder, Recorder};
use crate::tool::Tool;

/// What an adapter crate hands the CLI to register. Its name is its scene
/// compiler's kind, so its halves cannot be registered under two.
pub struct Adapter {
    scene: Box<dyn SceneCompiler>,
    capture: Box<dyn CaptureBackend>,
    recorder: Option<Box<dyn Recorder>>,
}

impl Adapter {
    pub fn new(
        scene: impl SceneCompiler + 'static,
        capture: impl CaptureBackend + 'static,
    ) -> Self {
        Self {
            scene: Box::new(scene),
            capture: Box::new(capture),
            recorder: None,
        }
    }

    /// The adapter, also recording sessions with `recorder`.
    pub fn recorded_with(mut self, recorder: impl Recorder + 'static) -> Self {
        self.recorder = Some(Box::new(recorder));
        self
    }

    /// The name a block's scene gives.
    pub fn name(&self) -> &'static str {
        self.scene.kind()
    }

    pub fn scene(&self) -> &dyn SceneCompiler {
        self.scene.as_ref()
    }

    pub fn capture(&self) -> &dyn CaptureBackend {
        self.capture.as_ref()
    }

    /// Its recorder, if its tool records sessions.
    pub fn into_recorder(self) -> Option<NamedRecorder> {
        let adapter = self.name();
        self.recorder
            .map(|recorder| NamedRecorder { adapter, recorder })
    }

    /// What it runs, capturing and then recording, each once: what
    /// `teleprompt setup <adapter>` installs.
    pub fn needs(&self) -> Vec<&'static Tool> {
        let recording = self.recorder.as_ref().map_or(&[][..], |r| r.needs());
        let mut out: Vec<&'static Tool> = Vec::new();
        for &tool in self.capture.needs().iter().chain(recording) {
            if !out.iter().any(|t| t.name == tool.name) {
                out.push(tool);
            }
        }
        out
    }

    /// Its scene compiler, for the registry `compile` reads.
    pub fn into_scene(self) -> Box<dyn SceneCompiler> {
        self.scene
    }

    /// Its capture backend, for the registry `capture` runs.
    pub fn into_capture(self) -> Box<dyn CaptureBackend> {
        self.capture
    }
}
