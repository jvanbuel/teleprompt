use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};

const PROJECT_TOML: &str = r#"# teleprompt project configuration
[locales]
source = "en"
targets = []

[voice]
source = "synthetic"
backend = "null"

[scene.mock]
adapter = "mock"
"#;

const DEMO_SCRIPT: &str = r#"---
teleprompt: 1
scene:
  mock:
    adapter: mock
---

# Getting started

Welcome to teleprompt. This paragraph is a narration segment, and its spoken
length decides how long the visuals below stay on screen. {#welcome}

```teleprompt scene=mock
wait 500ms
```

Edit that sentence, run `teleprompt diff`, and watch every later transition
move. That feedback loop is the whole point. {#the-loop}

```teleprompt scene=mock policy=concurrent
wait 800ms
```
"#;

const GITIGNORE: &str = ".teleprompt/cache/\n.teleprompt/traces/\nbuild/\ntakes/*.wav\n";

pub fn scaffold(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let manifest = dir.join("teleprompt.toml");
    if manifest.exists() {
        return Err(Error::new(
            ErrorKind::AlreadyExists,
            format!("{} already contains a teleprompt project", dir.display()),
        ));
    }

    std::fs::create_dir_all(dir.join("scripts"))?;
    std::fs::create_dir_all(dir.join("timelines"))?;

    let files = [
        (manifest, PROJECT_TOML),
        (dir.join("scripts/demo.md"), DEMO_SCRIPT),
        (dir.join(".gitignore"), GITIGNORE),
    ];

    let mut written = Vec::new();
    for (path, contents) in files {
        std::fs::write(&path, contents)?;
        written.push(path);
    }
    Ok(written)
}
