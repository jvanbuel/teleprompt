use serde_json::{json, Value};
use teleprompt_testkit::http::{self, Reply, Stub};

use teleprompt_project::translate::{Claude, Program, Request, Translator, Wanted};

fn request() -> Request {
    Request {
        source: "en".into(),
        target: "nl".into(),
        existing: Vec::new(),
        translate: vec![Wanted {
            id: "line:welcome".into(),
            kind: "line",
            english: "Welcome to Acme.".into(),
            line: None,
        }],
    }
}

/// A server answering `reply`, and where it is.
async fn serve_once(reply: Value) -> (String, Stub) {
    let stub = http::spawn(Reply::json(reply)).await;
    (stub.base_url.clone(), stub)
}

/// The head and JSON body of the request `server` was sent.
fn asked(server: &Stub) -> (String, Value) {
    let request = server.first();
    (request.head.clone(), request.json())
}

fn claude(url: String) -> Translator {
    Translator::Claude(Claude {
        api_key: "test-key".into(),
        model: "claude-opus-5".into(),
        base_url: url,
        timeout_ms: 600_000,
    })
}

#[tokio::test]
async fn claude_is_asked_for_json_and_its_answer_read() {
    let answer = json!({"items": [{"id": "line:welcome", "text": "Welkom bij Acme."}]});
    let (url, server) = serve_once(json!({
        "stop_reason": "end_turn",
        "content": [{"type": "text", "text": answer.to_string()}],
    }))
    .await;
    let out = claude(url).translate(&request()).await.unwrap();
    assert_eq!(
        out,
        [("line:welcome".to_string(), "Welkom bij Acme.".to_string())]
    );

    let (head, body) = asked(&server);
    let head = head.to_lowercase();
    assert!(head.starts_with("post /v1/messages "), "{head}");
    assert!(head.contains("x-api-key: test-key") && head.contains("anthropic-version: 2023-06-01"));
    assert!(head.contains("anthropic-beta: server-side-fallback-2026-07-01"));
    assert_eq!(body["model"], "claude-opus-5");
    assert_eq!(body["fallbacks"], "default");
    assert_eq!(body["output_config"]["format"]["type"], "json_schema");
    let asked: Value =
        serde_json::from_str(body["messages"][0]["content"].as_str().unwrap()).unwrap();
    assert_eq!(asked["target"], "nl");
    assert_eq!(asked["translate"][0]["english"], "Welcome to Acme.");
}

#[tokio::test]
async fn a_refusal_is_an_error_not_an_empty_translation() {
    let (url, _server) = serve_once(json!({
        "stop_reason": "refusal",
        "stop_details": {"type": "refusal", "explanation": "no"},
        "content": [],
    }))
    .await;
    let err = claude(url).translate(&request()).await.unwrap_err();
    assert!(err.contains("declined"), "{err}");
}

#[tokio::test]
async fn a_command_gets_the_request_on_stdin_and_answers_on_stdout() {
    // Echoes the first item's English back as its translation.
    let script = r#"python3 -c 'import json,sys; r=json.load(sys.stdin); i=r["translate"][0]; print(json.dumps({"items":[{"id":i["id"],"text":r["target"]+": "+i["english"]}]}))'"#;
    let out = Translator::Command(Program::new(script))
        .translate(&request())
        .await
        .unwrap();
    assert_eq!(
        out,
        [(
            "line:welcome".to_string(),
            "nl: Welcome to Acme.".to_string()
        )]
    );
}

#[tokio::test]
async fn a_failing_command_says_what_it_said() {
    let err = Translator::Command(Program::new("echo nope >&2; exit 3"))
        .translate(&request())
        .await
        .unwrap_err();
    assert!(err.contains("nope") && err.contains('3'), "{err}");
}

fn settings(yaml: &str) -> serde_yaml::Value {
    serde_yaml::from_str(yaml).unwrap()
}

/// The default: a model run by Ollama, asked for JSON in its own format.
#[tokio::test]
async fn ollama_is_the_default_and_asked_for_json() {
    let answer = json!({"items": [{"id": "line:welcome", "text": "Welkom bij Acme."}]});
    let (url, server) = serve_once(json!({
        "message": {"role": "assistant", "content": format!("```json\n{answer}\n```")},
        "done": true,
    }))
    .await;
    let t = Translator::new(
        teleprompt_project::translate::PROVIDERS[0],
        None,
        Some(&settings(&format!("url: {url}"))),
    )
    .unwrap();
    let out = t.translate(&request()).await.unwrap();
    assert_eq!(
        out,
        [("line:welcome".to_string(), "Welkom bij Acme.".to_string())]
    );

    let (head, body) = asked(&server);
    assert!(head.to_lowercase().starts_with("post /api/chat "), "{head}");
    assert_eq!(
        body["model"],
        teleprompt_project::translate::ollama::DEFAULT_MODEL
    );
    assert_eq!(body["stream"], false);
    assert_eq!(body["format"]["required"][0], "items");
    assert_eq!(body["messages"][0]["role"], "system");
}

/// A model Ollama has not pulled says how to get it.
#[tokio::test]
async fn a_missing_ollama_model_says_how_to_pull_it() {
    let (url, _server) =
        serve_once(json!({"error": "model \"gemma3:12b\" not found, try pulling it first"})).await;
    let t = Translator::new("ollama", None, Some(&settings(&format!("url: {url}")))).unwrap();
    let err = t.translate(&request()).await.unwrap_err();
    assert!(err.contains("ollama pull gemma3:12b"), "{err}");
}

/// No Ollama running is an error naming what to install, not a hang.
#[tokio::test]
async fn no_ollama_says_how_to_get_one() {
    let t = Translator::new("ollama", None, Some(&settings("url: http://127.0.0.1:9"))).unwrap();
    let err = t.translate(&request()).await.unwrap_err();
    assert!(
        err.contains("ollama.com") && err.contains("[translate]"),
        "{err}"
    );
}

/// Any OpenAI-compatible server: LM Studio, llama.cpp, vLLM.
#[tokio::test]
async fn an_openai_compatible_server_is_asked_the_same() {
    let answer = json!({"items": [{"id": "line:welcome", "text": "Welkom bij Acme."}]});
    let (url, server) = serve_once(json!({
        "choices": [{"message": {"role": "assistant", "content": answer.to_string()}}],
    }))
    .await;
    // A variable cargo sets for every test run: setting one here would
    // race the other tests reading the environment in parallel.
    let t = Translator::new(
        "openai",
        Some("qwen2.5-7b-instruct"),
        Some(&settings(&format!(
            "url: {url}/v1\napi_key_env: CARGO_PKG_NAME"
        ))),
    )
    .unwrap();
    let out = t.translate(&request()).await.unwrap();
    assert_eq!(out.len(), 1);
    let (head, body) = asked(&server);
    let head = head.to_lowercase();
    assert!(head.starts_with("post /v1/chat/completions "), "{head}");
    assert!(
        head.contains("authorization: bearer teleprompt-project"),
        "{head}"
    );
    assert_eq!(body["model"], "qwen2.5-7b-instruct");
    assert_eq!(body["response_format"]["type"], "json_schema");
}

#[test]
fn providers_say_what_they_are_missing() {
    let err = |p: &str, s: Option<&str>| {
        Translator::new(p, None, s.map(settings).as_ref())
            .err()
            .unwrap()
            .to_string()
    };
    assert!(err("deepl", None).contains("ollama, openai, claude, command"));
    assert!(err("openai", None).contains("[translate.openai]"));
    assert!(err("command", None).contains("[translate.command]"));
    assert!(err("ollama", Some("adress: x")).contains("translate.ollama"));
}

/// A server that takes the request and never answers, as a wedged model
/// or a black-holing proxy does.
async fn silent() -> (String, Stub) {
    let stub = http::spawn(Reply::Hang).await;
    (stub.base_url.clone(), stub)
}

/// A provider that never answers is given up on after `[translate]`'s
/// `timeout_ms`, whichever provider it is, saying so rather than hanging
/// `translate` for ever.
#[tokio::test]
async fn a_provider_that_never_answers_is_given_up_on() {
    let mut translators = Vec::new();
    for provider in ["ollama", "openai"] {
        let (url, held) = silent().await;
        let settings = settings(&format!("url: {url}"));
        let t = Translator::new(provider, Some("m"), Some(&settings)).unwrap();
        translators.push((provider, t, Some(held)));
    }
    let (url, held) = silent().await;
    translators.push(("claude", claude(url), Some(held)));
    translators.push((
        "command",
        Translator::Command(Program::new("sleep 30")),
        None,
    ));
    for (provider, t, _held) in translators {
        let started = std::time::Instant::now();
        let err = t.timeout(300).translate(&request()).await.unwrap_err();
        assert!(started.elapsed().as_secs() < 5, "{provider}");
        assert!(
            err.contains("`timeout_ms` under [translate]"),
            "{provider}: {err}"
        );
    }
}

/// A translator printed for debugging does not print its key.
#[test]
fn a_key_is_never_printed() {
    let Translator::Claude(c) = claude("http://localhost".into()) else {
        unreachable!()
    };
    let shown = format!("{c:?}");
    assert!(
        !shown.contains("test-key") && shown.contains("redacted"),
        "{shown}"
    );
}
