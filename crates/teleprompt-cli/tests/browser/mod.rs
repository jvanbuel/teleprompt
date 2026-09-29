// A page as a browser leaves it once its scripts have run: headless Chromium
// with `--dump-dom`. Tests that need it skip on a machine without one, as the
// ffmpeg tests do without ffmpeg.
#![allow(dead_code)]

use std::path::PathBuf;
use std::process::Command;

/// `TELEPROMPT_CHROMIUM`, a Chromium on PATH, or Playwright's.
pub fn chromium() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("TELEPROMPT_CHROMIUM") {
        return Some(PathBuf::from(path));
    }
    for name in ["chromium", "chromium-browser", "google-chrome", "chrome"] {
        let found = Command::new("sh")
            .args(["-c", &format!("command -v {name}")])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()));
        if found.is_some() {
            return found;
        }
    }
    let root = std::env::var_os("PLAYWRIGHT_BROWSERS_PATH")?;
    std::fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|e| e.path().join("chrome-linux/chrome"))
        .find(|p| p.exists())
}

/// The DOM of `url` after `ms` of the page's own time, at `width` by `height`.
pub fn dom(chrome: &std::path::Path, url: &str, width: u32, height: u32, ms: u32) -> String {
    let out = Command::new(chrome)
        .args([
            "--headless=new",
            "--no-sandbox",
            "--disable-gpu",
            "--dump-dom",
            &format!("--virtual-time-budget={ms}"),
            &format!("--window-size={width},{height}"),
            url,
        ])
        .output()
        .expect("chromium runs");
    String::from_utf8_lossy(&out.stdout).into_owned()
}
