//! What completion offers where the cursor is: a speaker's label opening
//! a paragraph, a line's attributes in its braces, a chapter's settings on
//! its heading, a block's attributes on its fence, and each one's values.

use lsp_types::Position;
use teleprompt::lsp::complete::{context, items, Context};
use teleprompt::lsp::{Definition, Project};

const SCRIPT: &str =
    "---\nteleprompt: 1\n---\n\n# Tour {speaker=g}\n**Gu\n\nHello there. {#hi }\n\n\
                      ```teleprompt scene=te policy=co\n```\n\nPlain text **here.\n";

fn at(needle: &str, after: usize) -> Position {
    let offset = SCRIPT.find(needle).unwrap() + after;
    teleprompt::lsp::text::LineIndex::new(SCRIPT).position(offset)
}

fn project() -> Project {
    let def = |name: &str, detail: &str| Definition {
        name: name.into(),
        detail: detail.into(),
        location: None,
    };
    Project {
        speakers: vec![def("guest", "gemini · Puck"), def("me", "kokoro · am_adam")],
        scenes: vec![def("terminal", "vhs"), def("browser", "playwright")],
        script_dir: std::env::temp_dir(),
    }
}

fn labels(ctx: &Context) -> Vec<String> {
    items(ctx, &project())
        .into_iter()
        .map(|i| i.label)
        .collect()
}

#[test]
fn a_speaker_is_offered_after_bold_opening_a_paragraph() {
    let (ctx, prefix) = context(SCRIPT, at("**Gu", 4)).unwrap();
    assert_eq!(ctx, Context::Speaker);
    assert_eq!(prefix, "Gu");
    assert_eq!(labels(&ctx), ["Guest", "Me"]);
    // Bold within a paragraph is not a label.
    assert!(context(SCRIPT, at("**here", 4)).is_none());
}

#[test]
fn a_headings_settings_are_offered_in_its_braces() {
    let (ctx, prefix) = context(SCRIPT, at("speaker=g", 9)).unwrap();
    assert_eq!(
        ctx,
        Context::Value {
            key: "speaker".into()
        }
    );
    assert_eq!(prefix, "g");
    assert_eq!(labels(&ctx), ["guest", "me"]);
    let (ctx, _) = context(SCRIPT, at("{speaker", 1)).unwrap();
    assert_eq!(ctx, Context::HeadingKey);
    assert!(labels(&ctx).contains(&"voice.speed".to_string()));
}

#[test]
fn a_lines_keys_are_offered_in_its_braces() {
    let (ctx, prefix) = context(SCRIPT, at("{#hi ", 5)).unwrap();
    assert_eq!((ctx.clone(), prefix.as_str()), (Context::LineKey, ""));
    let offered = labels(&ctx);
    for key in ["voice.instruct", "lead_in"] {
        assert!(offered.iter().any(|l| l == key), "{key} in {offered:?}");
    }
    assert!(!offered.iter().any(|l| l == "scene"), "{offered:?}");
}

#[test]
fn a_blocks_keys_and_values_are_offered_on_its_fence() {
    let (ctx, prefix) = context(SCRIPT, at("scene=te", 8)).unwrap();
    assert_eq!(
        ctx,
        Context::Value {
            key: "scene".into()
        }
    );
    assert_eq!(prefix, "te");
    assert_eq!(labels(&ctx), ["terminal", "browser"]);
    let (ctx, _) = context(SCRIPT, at("policy=co", 9)).unwrap();
    let policies = labels(&ctx);
    assert!(policies.contains(&"concurrent".to_string()), "{policies:?}");
    for policy in &policies {
        assert!(
            teleprompt_core::policy::PolicyKind::parse(policy).is_ok(),
            "{policy}"
        );
    }
    let (ctx, _) = context(SCRIPT, at("```teleprompt ", 14)).unwrap();
    assert_eq!(ctx, Context::BlockKey);
    assert!(labels(&ctx).contains(&"include".to_string()));
}

#[test]
fn plain_text_offers_nothing() {
    assert!(context(SCRIPT, at("Plain text", 5)).is_none());
    assert!(context(SCRIPT, at("# Tour", 3)).is_none());
    // Nor does a line's id.
    assert!(context(SCRIPT, at("{#hi", 3)).is_none());
}
