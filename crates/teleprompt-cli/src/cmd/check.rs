use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use teleprompt_cache::VoiceCache;
use teleprompt_compile::{compile, CompileOutput, VoiceContext};
use teleprompt_core::config::PartialConfig;
use teleprompt_core::ident::assign_ids;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::{resolve, Item};
use teleprompt_core::{Diagnostic, Diagnostics};
use teleprompt_scene::SceneRegistry;
use teleprompt_voice::VoiceBackend;
use teleprompt_voice_null::WpmEstimator;

use crate::project::Project;
use crate::voice::Backends;

/// Where a project's synthesis cache lives. Shared between `compile_script`
/// (which only ever reads it) and `dub` (which populates it), so the two
/// can never drift onto different directories and silently stop agreeing
/// about what is warm.
pub fn cache_root(project: &Project) -> PathBuf {
    project.root.join(".teleprompt").join("cache")
}

/// Shared front half of every command: read, parse, identify, resolve, compile.
/// Has no side effects — it never writes to disk.
///
/// Returns the resolved backend alongside the compilation because the two
/// must be the same one: `cache_key` is computed from `backend.id()` and
/// `capabilities().version`, so a caller that synthesizes with a *different*
/// backend writes that backend's audio into the cache under this one's key.
/// `dub` is the only caller that reads it.
///
/// It is returned separately rather than being a field on `CompileOutput`
/// because `CompileOutput` is what `check`, `plan`, and `diff` receive: a
/// backend hanging off it would put `synthesize` one `.` away from the inner
/// loop, which is the thing `VoiceContext` exists to make impossible.
pub fn compile_script(
    project: &Project,
    script: &Path,
    locale: &str,
) -> Result<(CompileOutput, Arc<dyn VoiceBackend>), Vec<String>> {
    // The project's own `backends:` settings, read before the script even
    // exists on disk — `compile_script_with` needs a registry to resolve
    // the script's chosen backend against, and that resolution happens
    // before this function knows whether the script's own front matter
    // carries a further `backends:` override. Front-matter-level backend
    // settings therefore do not reach construction here; only the
    // project's are real, not defaults.
    compile_script_with(&backends_of(project), project, script, locale)
}

/// The project's backends, built once from its settings and told which file
/// those settings came from so a diagnostic about them can point at it.
pub fn backends_of(project: &Project) -> Backends {
    let settings = project.config.backends.clone().unwrap_or_default();
    crate::voice::backends_for(&settings, &project.config_path().display().to_string())
}

/// [`compile_script`] against caller-supplied backends.
///
/// The seam exists so a test can register a backend this build does not
/// ship and watch the whole path — key, synthesis, cache, manifest — follow
/// it. That is the claim spec §4.1 makes, and it is not a claim a workspace
/// with exactly one real backend can otherwise check.
pub fn compile_script_with(
    backends: &Backends,
    project: &Project,
    script: &Path,
    locale: &str,
) -> Result<(CompileOutput, Arc<dyn VoiceBackend>), Vec<String>> {
    // Before the script is even read: a `backends:` key naming nothing this
    // build ships is wrong about the project, not about whatever is being
    // compiled, and it is wrong in the same way for every script in the
    // repository. It is reported here rather than in `compile_script` so
    // that `dub` — which goes through this function, not that one — cannot
    // reach a server on settings the author never successfully wrote.
    let config_diags = backends.diagnostics();
    if !config_diags.is_empty() {
        return Err(render(
            &Diagnostics(config_diags),
            &script.display().to_string(),
        ));
    }

    let name = script
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| script.display().to_string());
    let display = script.display().to_string();

    let src =
        std::fs::read_to_string(script).map_err(|e| vec![format!("cannot read {display}: {e}")])?;

    let mut parsed = parse_script(&src).map_err(|d| render(&d, &display))?;

    let id_diags = assign_ids(&mut parsed);
    if id_diags.iter().any(|d| d.is_error()) {
        return Err(render(&teleprompt_core::Diagnostics(id_diags), &display));
    }

    let program = resolve(
        &parsed,
        &name,
        locale,
        &project.config,
        &PartialConfig::default(),
    )
    .map_err(|d| render(&d, &display))?;

    // A script's own front matter can set `backends:` too — `Config`'s
    // field doc says so, and `resolve` above dutifully merges it into
    // `program.config.backends`. But the registry `compile_script` built
    // (or that a caller of `compile_script_with` supplied) was constructed
    // from the *project's* `backends:` alone, before this script was ever
    // read — nothing downstream of `resolve` can reach back and
    // reconstruct an already-built backend. Silently accepting the merge
    // and never mentioning the mismatch would be the same defect the
    // segment-backend check below exists to catch: an author-written value
    // the merge computes and the mechanism it is meant to affect never
    // sees. A warning, not an error: unlike a segment resolving to an
    // unregistered backend, the compile still produces something correct —
    // audio from the *project's* settings for that backend, just not the
    // script's requested variant of them.
    let base_backends = project.config.backends.clone().unwrap_or_default();
    let backend_override_diags =
        backend_override_warnings(&base_backends, &program.config.backends);

    // The *resolved* backend, not just a name: a script's own front matter
    // can override `voice.backend`, and this is what actually produces the
    // segment's audio, so `cache_key` and `backend_version` both need to
    // come from it rather than from the raw string.
    // A backend whose settings did not validate fails here and only here —
    // when it is the one this project actually resolves to. Constructing
    // every backend eagerly used to fail `check` and `plan` with exit 2 on a
    // `backend = "null"` project because of a Kokoro setting it would never
    // read. The error is the same one; what changed is who gets it.
    let backend = backends
        .resolve(&program.config.voice.backend)
        .map_err(|d| render(&Diagnostics(vec![d]), &display))?;

    // Delivery A supports exactly one backend per compile: `VoiceContext`
    // carries a single `backend_id` for the whole program. A narration item
    // can resolve to a different backend than the program's overall
    // resolved backend two ways: a segment attribute set it, or a
    // chapter's front matter did. `Item::Narration` does not retain which
    // layer supplied the value — `config` is already the fully merged
    // result — so the diagnostic describes the *effect* ("this segment
    // resolves to a backend other than the one in use") rather than
    // guessing which layer is at fault and telling the author to remove an
    // attribute that, for a chapter-level override, was never written.
    //
    // Grouped by the offending value and reported once per group, naming
    // every affected segment, so a chapter of twelve paragraphs under one
    // stray `voice: { backend: ... }` produces one error, not twelve.
    let mut offenders: std::collections::BTreeMap<
        String,
        Vec<(String, teleprompt_core::SourceSpan)>,
    > = std::collections::BTreeMap::new();
    for item in &program.items {
        if let Item::Narration {
            id, config, span, ..
        } = item
        {
            if config.voice.backend != program.config.voice.backend {
                offenders
                    .entry(config.voice.backend.clone())
                    .or_default()
                    .push((id.clone(), *span));
            }
        }
    }
    if !offenders.is_empty() {
        let diags: Vec<Diagnostic> = offenders
            .into_iter()
            .map(|(backend, segments)| {
                let span = segments[0].1;
                let names = segments
                    .iter()
                    .map(|(id, _)| format!("`{id}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let (noun, verb) = if segments.len() == 1 {
                    ("segment", "resolves")
                } else {
                    ("segments", "resolve")
                };
                Diagnostic::error(format!(
                    "{noun} {names} {verb} to voice backend `{backend}`, but this compile uses \
                     `{}`",
                    program.config.voice.backend
                ))
                .at(span)
                .with_help(
                    "per-segment and per-chapter voice backends are not supported yet; set \
                     voice.backend at the project or script front-matter level instead",
                )
            })
            .collect();
        return Err(render(&Diagnostics(diags), &display));
    }

    // `script_dir`, not `script.parent()`: a bare `demo.md` has
    // `Some("")` for a parent, which is not the current directory.
    let base_dir = crate::project::script_dir(script);

    let cache = VoiceCache::new(cache_root(project));
    let estimator = WpmEstimator::default();
    let capabilities = backend.capabilities();
    let ctx = VoiceContext {
        backend_id: backend.id(),
        backend_version: &capabilities.version,
        cache: &cache,
        estimator: &estimator,
    };

    let mut out = compile(
        &program,
        &SceneRegistry::with_builtins(),
        &ctx,
        base_dir,
        env!("CARGO_PKG_VERSION"),
    )
    .map_err(|d| render(&d, &display))?;

    // The bare message, not `render()`'s output: every other entry in
    // `out.warnings` (cache, scheduling) is a plain sentence with no
    // severity prefix and no `--> file` line, because both callers of this
    // vec (`main.rs`'s `eprintln!("warning: {w}")` and `--format json`'s
    // `warnings` array) add their own framing on top. Pushing a
    // pre-rendered `"warning: ...\n  --> ..."` string here doubled the
    // prefix on stderr and made the JSON array mix two shapes.
    out.warnings
        .extend(backend_override_diags.into_iter().map(|d| d.message));

    Ok((out, backend))
}

/// Diagnoses `Config.backends` entries that `resolve` computed from a
/// script's own front matter but that a caller-supplied registry cannot
/// see — see the call site's comment for why. `base` is what the registry
/// was (or should have been) built from; `merged` is `program.config
/// .backends` after `resolve`. One diagnostic per distinct offending
/// backend id, not per differing key, mirroring the segment-backend
/// check's "once per group" shape.
fn backend_override_warnings(
    base: &std::collections::BTreeMap<String, serde_yaml::Value>,
    merged: &std::collections::BTreeMap<String, serde_yaml::Value>,
) -> Vec<Diagnostic> {
    merged
        .iter()
        .filter(|(id, value)| base.get(*id) != Some(*value))
        .map(|(id, value)| {
            let keys = differing_backend_keys(base.get(id), value);
            let which = if keys.is_empty() {
                String::new()
            } else {
                format!(" (`{}`)", keys.join("`, `"))
            };
            Diagnostic::warning(format!(
                "this script's front matter sets `backends.{id}`{which}, but backend settings \
                 are resolved once per project before any script is parsed, so the override has \
                 no effect here; set it in `teleprompt.toml` under `backends.{id}` instead"
            ))
        })
        .collect()
}

/// The mapping keys `after` sets or changes relative to `before`, when both
/// are YAML mappings. Empty for a non-mapping value (a wholesale
/// replacement — there is nothing granular to name) or when `before` is
/// absent (every key is "new", which the caller's message already covers
/// by naming the backend itself).
fn differing_backend_keys(
    before: Option<&serde_yaml::Value>,
    after: &serde_yaml::Value,
) -> Vec<String> {
    let (Some(before), Some(after)) = (before.and_then(|v| v.as_mapping()), after.as_mapping())
    else {
        return Vec::new();
    };
    after
        .iter()
        .filter(|(k, v)| before.get(*k) != Some(*v))
        .filter_map(|(k, _)| k.as_str().map(str::to_string))
        .collect()
}

/// Returns warnings on success, rendered errors on failure. `check` is
/// `compile_script` minus the timeline: parse, validate, compile — but the
/// caller only ever looks at whether it succeeded and what it has to say.
pub fn run_check(
    project: &Project,
    script: &Path,
    locale: &str,
) -> Result<Vec<String>, Vec<String>> {
    compile_script(project, script, locale).map(|(out, _)| out.warnings)
}

fn render(d: &teleprompt_core::Diagnostics, file: &str) -> Vec<String> {
    d.0.iter().map(|x| x.render(file)).collect()
}

/// The stable, typed shape of `check`'s `--format json` output. Mirrors
/// `DoctorReport`'s and `NewReport`'s pattern rather than building JSON
/// inline in `main.rs`: `ok` says which of `warnings`/`errors` is
/// meaningful, but both fields are always present so consumers never have
/// to branch on a missing key.
#[derive(Debug, Serialize)]
pub struct CheckReport {
    pub ok: bool,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_report_serializes_ok_and_both_lists() {
        let report = CheckReport {
            ok: true,
            warnings: vec!["bare wait".to_string()],
            errors: vec![],
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"ok": true, "warnings": ["bare wait"], "errors": []})
        );
    }
}
