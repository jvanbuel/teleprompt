//! The composition root: which adapters a built `teleprompt` can actually
//! reach.
//!
//! `SceneRegistry::with_builtins()` carries only the adapters inside
//! `teleprompt-scene`, so an adapter living in its own crate is reachable
//! only if something adds it. That something is `scene::scenes()`, and these
//! tests are what catch a command compiling against the narrower registry —
//! which fails `scene=terminal` with "no adapter `vhs` is available" on a
//! build that ships one.

use teleprompt_cli::scene::scenes;
use teleprompt_scene::SceneCompiler;

#[test]
fn the_registry_serves_every_adapter_this_build_ships() {
    let r = scenes();

    assert_eq!(r.get("mock").map(SceneCompiler::kind), Some("mock"));
    assert_eq!(r.get("vhs").map(SceneCompiler::kind), Some("vhs"));
    assert_eq!(r.available(), vec!["mock", "vhs"]);
}

#[test]
fn an_unknown_adapter_is_not_served() {
    assert!(scenes().get("playwright").is_none());
}
