//! The server, driven over an in-memory connection as an editor drives it,
//! with an analyzer that stands in for the compiler.

use std::path::{Path, PathBuf};

use lsp_server::{Connection, Message, Notification, Request, RequestId};
use lsp_types::Url;
use serde_json::{json, Value};
use teleprompt_core::{Diagnostic, SourceSpan};
use teleprompt_lsp::{Analysis, Analyzer, Definition, Project};

struct Stub {
    toml: PathBuf,
}

impl Analyzer for Stub {
    fn project(&self, _script: &Path) -> Project {
        Project {
            speakers: vec![Definition {
                name: "guest".into(),
                detail: "gemini · Puck".into(),
                location: Some((self.toml.clone(), 7)),
            }],
            scenes: Vec::new(),
            script_dir: std::env::temp_dir(),
        }
    }

    fn analyze(&self, _script: &Path, text: &str) -> Analysis {
        let mut analysis = Analysis::default();
        if text.contains("**Gust:**") {
            analysis.diagnostics.push(
                Diagnostic::error("no speaker `gust` in the cast (`guest`)").at(SourceSpan {
                    line: 7,
                    column: 1,
                    len: 6,
                }),
            );
        }
        analysis.hovers.insert(7, "**line `hi`** · 1.2 s".into());
        analysis
    }
}

const SCRIPT: &str = "---\nteleprompt: 1\n---\n\n# Tour\n\n**Gust:** Hello. {#hi}\n";

struct Editor {
    conn: Connection,
    next: i32,
}

impl Editor {
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = RequestId::from(self.next);
        self.conn
            .sender
            .send(Message::Request(Request::new(
                id.clone(),
                method.into(),
                params,
            )))
            .unwrap();
        loop {
            match self.conn.receiver.recv().unwrap() {
                Message::Response(r) if r.id == id => return r.result.unwrap_or(Value::Null),
                _ => continue,
            }
        }
    }

    fn notify(&self, method: &str, params: Value) {
        self.conn
            .sender
            .send(Message::Notification(Notification::new(
                method.into(),
                params,
            )))
            .unwrap();
    }

    /// The next notification of `method`.
    fn wait(&self, method: &str) -> Value {
        loop {
            if let Message::Notification(n) = self.conn.receiver.recv().unwrap() {
                if n.method == method {
                    return n.params;
                }
            }
        }
    }
}

fn started(script: &str) -> (Editor, Url, std::thread::JoinHandle<()>) {
    let (client, server) = Connection::memory();
    let toml = std::env::temp_dir().join("teleprompt-lsp-test.toml");
    let handle = std::thread::spawn(move || {
        teleprompt_lsp::serve(server, Stub { toml }).unwrap();
    });
    let mut editor = Editor {
        conn: client,
        next: 0,
    };
    let caps = editor.request("initialize", json!({ "capabilities": {} }));
    assert!(
        caps["capabilities"]["completionProvider"].is_object(),
        "{caps}"
    );
    editor.notify("initialized", json!({}));
    let uri = Url::from_file_path(std::env::temp_dir().join("tour.md")).unwrap();
    editor.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": uri, "languageId": "markdown", "version": 1, "text": script } }),
    );
    (editor, uri, handle)
}

fn stop(mut editor: Editor, handle: std::thread::JoinHandle<()>) {
    editor.request("shutdown", Value::Null);
    editor.notify("exit", Value::Null);
    handle.join().unwrap();
}

#[test]
fn problems_are_published_where_they_are_and_cleared_when_fixed() {
    let (editor, uri, handle) = started(SCRIPT);
    let published = editor.wait("textDocument/publishDiagnostics");
    assert_eq!(published["uri"], json!(uri));
    let d = &published["diagnostics"][0];
    assert_eq!(d["severity"], 1);
    assert_eq!(d["source"], "teleprompt");
    assert_eq!(d["range"]["start"], json!({ "line": 6, "character": 0 }));
    assert!(d["message"].as_str().unwrap().contains("gust"));
    editor.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{ "text": SCRIPT.replace("**Gust:**", "**Guest:**") }] }),
    );
    let published = editor.wait("textDocument/publishDiagnostics");
    assert_eq!(published["diagnostics"], json!([]));
    stop(editor, handle);
}

#[test]
fn completion_hover_definition_and_symbols_answer() {
    let (mut editor, uri, handle) = started(SCRIPT);
    editor.wait("textDocument/publishDiagnostics");
    let doc = json!({ "uri": uri });
    // After `**Gu` of `**Gust:**`: the cast, as labels.
    let items = editor.request(
        "textDocument/completion",
        json!({ "textDocument": doc, "position": { "line": 6, "character": 4 } }),
    );
    assert_eq!(items[0]["label"], "Guest", "{items}");
    assert_eq!(items[0]["insertText"], "Guest:** ", "{items}");
    let hover = editor.request(
        "textDocument/hover",
        json!({ "textDocument": doc, "position": { "line": 6, "character": 12 } }),
    );
    assert!(
        hover["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("line `hi`"),
        "{hover}"
    );
    let symbols = editor.request(
        "textDocument/documentSymbol",
        json!({ "textDocument": doc }),
    );
    assert_eq!(symbols[0]["name"], "Tour", "{symbols}");
    assert_eq!(symbols[0]["children"][0]["name"], "hi", "{symbols}");
    stop(editor, handle);
    // On a speaker's name: where the cast names them.
    let (mut editor, uri, handle) = started(&SCRIPT.replace("**Gust:**", "**Guest:**"));
    let definition = editor.request(
        "textDocument/definition",
        json!({ "textDocument": { "uri": uri }, "position": { "line": 6, "character": 4 } }),
    );
    assert!(
        definition["uri"]
            .as_str()
            .unwrap()
            .ends_with("teleprompt-lsp-test.toml"),
        "{definition}"
    );
    assert_eq!(definition["range"]["start"]["line"], 7);
    stop(editor, handle);
}

/// An editor sends every Markdown file; one that is not a script, such as
/// a README, is left alone.
#[test]
fn a_markdown_file_that_is_not_a_script_is_left_alone() {
    let (editor, _uri, handle) = started("# Notes\n\n**Gust:** here.\n");
    let published = editor.wait("textDocument/publishDiagnostics");
    assert_eq!(published["diagnostics"], json!([]));
    stop(editor, handle);
    assert!(teleprompt_lsp::is_script(
        "---\nteleprompt: 1\n---\n\n# A\n"
    ));
    assert!(teleprompt_lsp::is_script(
        "# A\n\nHi.\n\n```teleprompt scene=mock\n```\n"
    ));
    assert!(!teleprompt_lsp::is_script(
        "---\ntitle: Notes\n---\n\n# A\n"
    ));
}
