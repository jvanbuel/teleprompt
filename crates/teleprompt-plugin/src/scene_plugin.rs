//! A scene plugin, whole: how its blocks compile, how its scenes are captured
//! and, where its tool can, how an author's session is recorded, under the
//! one name a block's scene gives (`docs/design.md#crates`).

use crate::scene::{SceneCompiler, SceneCompilers};

use crate::capture::mock::MockCapture;
use crate::capture::CaptureBackend;
use crate::record::Recorder;
use crate::scene::MockScene;
use teleprompt_core::tool::Tool;

/// What a scene plugin crate hands the CLI to register. Its name is its scene
/// compiler's kind, so its halves cannot be registered under two.
pub struct ScenePlugin {
    scene: Box<dyn SceneCompiler>,
    capture: Box<dyn CaptureBackend>,
    recorder: Option<Box<dyn Recorder>>,
}

impl ScenePlugin {
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

    /// The plugin, also recording sessions with `recorder`.
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
    pub fn recorder(&self) -> Option<&dyn Recorder> {
        self.recorder.as_deref()
    }

    /// What it runs, capturing and then recording, each once: what
    /// `teleprompt setup <plugin>` installs.
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
}

/// Every scene plugin a command can use, by name: what `compile` reads a
/// block's shots from, what `capture` records with, and what `record`
/// records an author with.
pub struct ScenePlugins {
    plugins: Vec<ScenePlugin>,
}

impl ScenePlugins {
    /// `plugins`, in the order `setup` and errors list them; the first of
    /// a name wins.
    pub fn new(plugins: impl IntoIterator<Item = ScenePlugin>) -> Self {
        let mut out: Vec<ScenePlugin> = Vec::new();
        for plugin in plugins {
            if !out.iter().any(|p| p.name() == plugin.name()) {
                out.push(plugin);
            }
        }
        Self { plugins: out }
    }

    /// The mock alone: scene plugin crates depend on this one, so the
    /// real ones are put together by the CLI.
    pub fn mock() -> Self {
        Self::new([ScenePlugin::new(MockScene, MockCapture::default())])
    }

    /// These, and `plugin` unless one has its name.
    pub fn with(self, plugin: ScenePlugin) -> Self {
        Self::new(self.plugins.into_iter().chain([plugin]))
    }

    pub fn get(&self, name: &str) -> Option<&ScenePlugin> {
        self.plugins.iter().find(|p| p.name() == name)
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.plugins.iter().map(ScenePlugin::name).collect()
    }

    pub fn iter(&self) -> impl Iterator<Item = &ScenePlugin> {
        self.plugins.iter()
    }
}

impl SceneCompilers for ScenePlugins {
    fn compiler(&self, name: &str) -> Option<&dyn SceneCompiler> {
        self.get(name).map(ScenePlugin::scene)
    }

    fn names(&self) -> Vec<&'static str> {
        ScenePlugins::names(self)
    }
}
