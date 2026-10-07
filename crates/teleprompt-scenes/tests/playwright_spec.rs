//! One test of a Playwright test file, as `include=file#title` names it.

use teleprompt_scene::contract::SceneCompiler;
use teleprompt_scene::core::BlockId;
use teleprompt_scenes::playwright::PlaywrightScene;

const SPEC: &str = r#"import { test, expect } from '@playwright/test';

test.describe('checkout', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
  });

  test('pays by card', async ({ page }) => {
    await page.getByRole('button', { name: 'Pay' }).click();
    // a brace in a comment: }
    await page.fill('#note', "a } in a string");
    // mark
    await expect(page.getByText(`Paid {ok}`)).toBeVisible();
  });

  test.only("pays by invoice", async ({ page }) => {
    await page.click('text=Invoice');
  });
});
"#;

#[test]
fn a_title_is_that_test_s_body() {
    let body = PlaywrightScene.select(SPEC, "pays by card").unwrap();
    assert_eq!(
        body,
        "await page.getByRole('button', { name: 'Pay' }).click();\n\
         // a brace in a comment: }\n\
         await page.fill('#note', \"a } in a string\");\n\
         // mark\n\
         await expect(page.getByText(`Paid {ok}`)).toBeVisible();\n"
    );
    let invoice = PlaywrightScene.select(SPEC, "pays by invoice").unwrap();
    assert_eq!(invoice, "await page.click('text=Invoice');\n");
}

#[test]
fn an_unknown_title_lists_the_file_s_tests() {
    let e = PlaywrightScene.select(SPEC, "refunds").unwrap_err();
    assert!(
        e.contains("`pays by card`") && e.contains("`pays by invoice`"),
        "{e}"
    );
    assert!(!e.contains("beforeEach"), "{e}");
}

#[test]
fn a_script_s_fragment_is_a_part_between_marks() {
    let script = "await page.goto('/');\n// mark\nawait page.click('a');\n";
    assert_eq!(
        PlaywrightScene.select(script, "2").unwrap(),
        "await page.click('a');\n"
    );
}

#[test]
fn a_whole_test_file_asks_for_a_test_to_be_named() {
    use teleprompt_scene::contract::{BlockSource, BodyOrigin};
    use teleprompt_scene::core::SourceSpan;
    let src = BlockSource {
        scene: "browser".into(),
        body: SPEC.into(),
        origin: BodyOrigin::Inline {
            fence: SourceSpan {
                line: 1,
                column: 1,
                len: 0,
            },
        },
    };
    let v = PlaywrightScene.validate(&src).unwrap();
    let e = PlaywrightScene.shots(&v, &BlockId::from("b")).unwrap_err();
    assert!(e[0].message.contains("name the test"), "{:?}", e[0]);
}
