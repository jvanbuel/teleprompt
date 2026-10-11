//! A written dub captures its own shots, into the clip store, in-process:
//! with a capture backend that writes its clips rather than records them.

use std::path::Path;

use teleprompt_core::Silent;
use teleprompt_project::build::FrameOverride;
use teleprompt_project::dub::Dubber;
use teleprompt_project::project::{Project, Script};
use teleprompt_project::registry::Registry;
use teleprompt_scene::capture::{CaptureBackend, CaptureError, Clip, Frame, Progress, Session};
use teleprompt_scene::{MockScene, ScenePlugin, ScenePlugins};

const SCRIPT: &str = "\
---
teleprompt: 1
scene:
  demo:
    plugin: mock
---

# A short tour

The scene below runs underneath this. {#opening}

```teleprompt scene=demo
wait 800ms
```

And a second paragraph. {#second}

```teleprompt scene=demo
wait 800ms
```
";

/// Writes each wanted shot's clip at the frame it is asked for.
struct Writes;

impl CaptureBackend for Writes {
    fn capture(
        &self,
        session: &Session,
        frame: &Frame,
        out_dir: &Path,
        _on_progress: &mut dyn FnMut(Progress),
    ) -> Result<Vec<Clip>, CaptureError> {
        std::fs::create_dir_all(out_dir).unwrap();
        Ok(session
            .shots
            .iter()
            .filter(|s| s.wanted)
            .map(|shot| {
                let path = out_dir.join(format!("{}.mp4", shot.key));
                std::fs::write(&path, format!("{}x{}", frame.width, frame.height)).unwrap();
                Clip {
                    key: shot.key,
                    path,
                }
            })
            .collect())
    }
}

#[tokio::test]
async fn a_dub_captures_what_the_store_lacks_and_reuses_the_rest() {
    let dir = teleprompt_testkit::test_dir("dub-capture");
    std::fs::write(dir.path().join("teleprompt.toml"), "").unwrap();
    let path = dir.path().join("tour.md");
    std::fs::write(&path, SCRIPT).unwrap();
    let project = Project::discover(dir.path(), Registry::mock()).unwrap();
    let script = Script::open(project, &path, "en");
    let plugins = ScenePlugins::new([ScenePlugin::new(MockScene, Writes)]);
    let clips = script.project().caches().clips();
    let frame = FrameOverride {
        resolution: Some((64, 36)),
        fps: None,
    };

    let dubbed = Dubber::new(&script)
        .dub(&dir.path().join("out"))
        .await
        .unwrap();
    let first = dubbed.capture(&plugins, &clips, frame, &Silent);
    let again = dubbed.capture(&plugins, &clips, frame, &Silent);

    assert_eq!(
        (first.captured, first.reused),
        (2, 0),
        "{:?}",
        first.warnings
    );
    assert_eq!((again.sessions, again.reused), (0, 2));
    for shot in &dubbed.manifest.shots {
        let clip = clips.path_of(&shot.capture_key).expect("captured");
        assert_eq!(
            std::fs::read_to_string(clip).unwrap(),
            "64x36",
            "at the overridden frame"
        );
    }
}
