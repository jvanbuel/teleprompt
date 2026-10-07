//! A script `playwright codegen` wrote, as the steps a draft includes.

use teleprompt_scene::contract::SceneCompiler;
use teleprompt_scenes::playwright::record::{statements, timed};
use teleprompt_scenes::playwright::PlaywrightScene;

/// As `playwright codegen --target javascript` writes it.
const CODEGEN: &str = "const { chromium } = require('playwright');

(async () => {
  const browser = await chromium.launch({
    headless: false
  });
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto('https://example.com/');
  await page.getByRole('link', { name: 'More' }).click();

  // ---------------------
  await context.close();
  await browser.close();
})();
";

#[test]
fn the_statements_are_what_the_page_did() {
    assert_eq!(
        statements(CODEGEN),
        [
            "await page.goto('https://example.com/');",
            "await page.getByRole('link', { name: 'More' }).click();"
        ]
    );
}

#[test]
fn each_statement_is_a_step_at_the_time_it_appeared() {
    let r = timed(&statements(CODEGEN), &[400, 2600]);
    let starts: Vec<u64> = r.steps.iter().map(|s| s.start_ms).collect();
    assert_eq!(starts, [400, 2600]);
    let marked = r.marked(&[1]);
    assert_eq!(
        PlaywrightScene.select(&marked, "2").unwrap(),
        "await page.getByRole('link', { name: 'More' }).click();\n"
    );
}

/// codegen rewrites its last statement as the author types into a field:
/// the statement keeps the time it first appeared. One taken back and
/// replaced is a new step, at the time of its replacement.
#[test]
fn a_statement_keeps_the_time_it_first_appeared() {
    use std::time::Duration;
    use teleprompt_scenes::playwright::record::Seen;
    let s = |xs: &[&str]| xs.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let mut seen = Seen::default();
    seen.update(&s(&["goto"]), Duration::from_millis(100));
    seen.update(&s(&["goto", "fill('h')"]), Duration::from_millis(500));
    seen.update(&s(&["goto", "fill('hello')"]), Duration::from_millis(900));
    assert_eq!(seen.times, [100, 500]);
    seen.update(&s(&["goto"]), Duration::from_millis(1000));
    seen.update(&s(&["goto", "click()"]), Duration::from_millis(1200));
    assert_eq!(seen.times, [100, 1200]);
}
