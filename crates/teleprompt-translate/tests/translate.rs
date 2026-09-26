use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use teleprompt_translate::{Claude, Request, Translator, Wanted};

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

/// Serves one request with `reply`, returning what was asked.
async fn serve_once(reply: Value) -> (String, tokio::task::JoinHandle<(String, Value)>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut raw = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            let n = socket.read(&mut buf).await.unwrap();
            raw.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&raw).to_string();
            if let Some(end) = text.find("\r\n\r\n") {
                let length: usize = text[..end]
                    .lines()
                    .find_map(|l| {
                        l.to_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse().unwrap())
                    })
                    .unwrap_or(0);
                if raw.len() >= end + 4 + length {
                    let head = text[..end].to_string();
                    let body: Value =
                        serde_json::from_slice(&raw[end + 4..end + 4 + length]).unwrap();
                    let out = reply.to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{out}",
                        out.len()
                    );
                    socket.write_all(response.as_bytes()).await.unwrap();
                    return (head, body);
                }
            }
        }
    });
    (url, handle)
}

fn claude(url: String) -> Translator {
    Translator::Claude(Claude {
        api_key: "test-key".into(),
        model: "claude-opus-5".into(),
        base_url: url,
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

    let (head, body) = server.await.unwrap();
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
    let out = Translator::Command(script.into())
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
    let err = Translator::Command("echo nope >&2; exit 3".into())
        .translate(&request())
        .await
        .unwrap_err();
    assert!(err.contains("nope") && err.contains('3'), "{err}");
}
