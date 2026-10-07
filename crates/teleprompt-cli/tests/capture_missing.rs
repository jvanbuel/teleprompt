//! A scene whose tools are not installed is said plainly, and says what to
//! do about it: capture goes on, and the shots become slates.

use std::process::Command;

const SCRIPT: &str = "\
---
teleprompt: 1
---

# Terminal

Here is a shell command.

```teleprompt scene=terminal
Type \"echo hi\"
Enter
```
";

#[test]
fn a_missing_tool_is_named_in_good_english_with_the_way_to_install_it() {
    let dir = teleprompt_testkit::test_dir("capture-missing");
    teleprompt_cli::commands::new::scaffold(&dir).unwrap();
    std::fs::write(dir.join("scripts/terminal.md"), SCRIPT).unwrap();
    let config = dir.join("teleprompt.toml");
    let mut text = std::fs::read_to_string(&config).unwrap();
    text.push_str("\n[scene.terminal]\nplugin = \"vhs\"\n");
    std::fs::write(&config, text).unwrap();
    let nothing = teleprompt_testkit::test_dir("capture-missing-path");

    let out = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .args(["capture", "scripts/terminal.md"])
        .current_dir(dir.path())
        .env("PATH", nothing.path())
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{said}");
    assert!(
        said.contains("vhs, ttyd and ffmpeg are not on PATH"),
        "{said}"
    );
    assert!(said.contains("`teleprompt setup vhs`"), "{said}");
    assert!(said.contains("render as slates"), "{said}");
}
