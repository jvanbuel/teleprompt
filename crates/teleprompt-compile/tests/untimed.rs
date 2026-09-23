//! A shot that states no length is given its sentence's. One with no
//! sentence would be given nothing, and is refused rather than dropped.

use std::path::Path;

use teleprompt_cache::VoiceCache;
use teleprompt_compile::{compile, VoiceContext};
use teleprompt_core::config::PartialConfig;
use teleprompt_core::ident::assign_ids;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::resolve;
use teleprompt_core::{Diagnostic, Diagnostics};
use teleprompt_scene::contract::{BlockSource, Measured, SceneCompiler, Shot, Validated};
use teleprompt_scene::mock::MockScene;
use teleprompt_scene::SceneRegistry;
use teleprompt_voice_null::WpmEstimator;

/// The mock's language, with no length claimed — as a composition or a
/// still states none.
struct Untimed;

impl SceneCompiler for Untimed {
    fn kind(&self) -> &'static str {
        "untimed"
    }
    fn validate(&self, src: &BlockSource) -> Result<Validated, Vec<Diagnostic>> {
        MockScene.validate(src)
    }
    fn shots(&self, v: &Validated, block_id: &str) -> Result<Vec<Shot>, Vec<Diagnostic>> {
        MockScene.shots(v, block_id)
    }
    fn estimate(&self, _shot: &Shot) -> Measured {
        Measured::Unknown
    }
}

fn compile_it(body: &str) -> Result<teleprompt_compile::CompileOutput, Diagnostics> {
    let src = format!("---\nscene: {{ s: {{ adapter: untimed }} }}\n---\n\n# A\n\n{body}");
    let cache =
        VoiceCache::new(std::env::temp_dir().join(format!("tp-untimed-{}", std::process::id())));
    let estimator = WpmEstimator::default();
    let ctx = VoiceContext {
        backend_id: "null",
        backend_version: "0.1.0",
        cache: &cache,
        estimator: &estimator,
    };
    let mut parsed = parse_script(&src).expect("parses");
    assign_ids(&mut parsed);
    let program = resolve(
        &parsed,
        "t.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .expect("resolves");
    let mut registry = SceneRegistry::with_builtins();
    registry.register(Box::new(Untimed));
    compile(&program, &registry, &ctx, Path::new("."), "0.1.0")
}

#[test]
fn a_shot_under_a_sentence_takes_the_sentence() {
    let out = compile_it("One sentence. {#a}\n\n```teleprompt scene=s\nwait 1ms\n```\n")
        .expect("compiles");
    assert!(out.timeline.entries[0].duration_ms > 0);
}

/// Before this was refused, the second shot was scheduled at 0 ms and
/// never reached the video, and `plan` printed a dash for it.
#[test]
fn a_shot_after_a_mark_is_refused_not_dropped() {
    let err =
        compile_it("One sentence. {#a}\n\n```teleprompt scene=s\nwait 1ms\nmark\nwait 1ms\n```\n")
            .expect_err("refused");
    assert_eq!(err.0.len(), 1, "{err:?}");
    assert!(err.0[0].message.contains("no sentence"), "{err:?}");
    assert!(err.0[0].help.as_deref().unwrap_or("").contains("paragraph"));
}
