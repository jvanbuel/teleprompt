//! A script `playwright codegen` wrote, as the steps a draft includes.

use teleprompt_playwright::record::{statements, timed};
use teleprompt_playwright::PlaywrightScene;
use teleprompt_scene::contract::SceneCompiler;

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
