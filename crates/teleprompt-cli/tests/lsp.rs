//! `teleprompt lsp`, run as an editor runs it: framed JSON-RPC on stdin
//! and stdout, against a real project, with the real compile behind it.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{json, Value};

struct Server {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next: i64,
}

impl Server {
    fn start(root: &std::path::Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
            .arg("lsp")
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
            next: 0,
        }
    }

    fn send(&mut self, message: Value) {
        let body = message.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        self.stdin.flush().unwrap();
    }

    fn receive(&mut self) -> Value {
        let mut length = 0;
        loop {
            let mut header = String::new();
            self.stdout.read_line(&mut header).unwrap();
            let header = header.trim();
            if header.is_empty() {
                break;
            }
            if let Some(n) = header.strip_prefix("Content-Length: ") {
                length = n.parse().unwrap();
            }
        }
        let mut body = vec![0; length];
        self.stdout.read_exact(&mut body).unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let message = self.receive();
            if message["id"] == id {
                return message["result"].clone();
            }
        }
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    fn diagnostics(&mut self) -> Value {
        loop {
            let message = self.receive();
            if message["method"] == "textDocument/publishDiagnostics" {
                return message["params"]["diagnostics"].clone();
            }
        }
    }
}

#[test]
fn an_editor_is_told_of_problems_and_offered_the_cast() {
    let dir = teleprompt_testkit::test_dir("lsp");
    teleprompt_project::new::scaffold(&dir).unwrap();
    let toml = dir.join("teleprompt.toml");
    let config = std::fs::read_to_string(&toml).unwrap();
    std::fs::write(
        &toml,
        format!("{config}\n[voices.guest]\nvoice = \"Puck\"\n"),
    )
    .unwrap();
    let script = dir.join("scripts/demo.md");
    let text = std::fs::read_to_string(&script)
        .unwrap()
        .replace("Welcome to teleprompt.", "**Gust:** Welcome to teleprompt.");
    let uri = format!("file://{}", script.display());

    let mut server = Server::start(&dir);
    let init = server.request("initialize", json!({ "capabilities": {} }));
    assert!(init["capabilities"]["hoverProvider"] == true, "{init}");
    server.notify("initialized", json!({}));
    server.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": uri, "languageId": "markdown", "version": 1, "text": text } }),
    );
    let problems = server.diagnostics();
    let problem = problems
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["message"].as_str().unwrap().contains("`Gust:`"))
        .unwrap_or_else(|| panic!("{problems}"));
    assert!(problem["message"].as_str().unwrap().contains("`guest`"));
    // Where the line is: the paragraph starts on the script's seventh line.
    assert_eq!(problem["range"]["start"]["line"], 6, "{problem}");

    // Fixed as it is typed, unsaved: the problem goes.
    let fixed = text.replace("**Gust:**", "**Guest:**");
    server.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": uri, "version": 2 }, "contentChanges": [{ "text": fixed }] }),
    );
    let left = server.diagnostics();
    let left = left.as_array().unwrap();
    assert!(
        left.iter()
            .all(|d| d["severity"] == 2 && !d["message"].as_str().unwrap().contains("Gu")),
        "{left:?}"
    );
    // The scaffold's own warning stays, on its line: `--check` read aloud.
    let the_loop = fixed
        .lines()
        .position(|l| l.starts_with("Edit that sentence"))
        .unwrap();
    assert!(
        left.iter().any(|d| d["range"]["start"]["line"] == the_loop
            && d["message"].as_str().unwrap().contains("--check")),
        "{left:?}"
    );

    let doc = json!({ "uri": uri });
    let welcome = fixed
        .lines()
        .position(|l| l.starts_with("**Guest:**"))
        .unwrap();
    let col = 4;
    let items = server.request(
        "textDocument/completion",
        json!({ "textDocument": doc, "position": { "line": welcome, "character": col } }),
    );
    assert_eq!(items[0]["label"], "Guest", "{items}");
    assert_eq!(items[0]["detail"], "null · Puck", "{items}");

    let hover = server.request(
        "textDocument/hover",
        json!({ "textDocument": doc, "position": { "line": 6, "character": 15 } }),
    );
    let said = hover["contents"]["value"].as_str().unwrap();
    assert!(
        said.contains("line `welcome`") && said.contains("said by guest"),
        "{said}"
    );

    let definition = server.request(
        "textDocument/definition",
        json!({ "textDocument": doc, "position": { "line": welcome, "character": col } }),
    );
    assert!(
        definition["uri"]
            .as_str()
            .unwrap()
            .ends_with("teleprompt.toml"),
        "{definition}"
    );
    let line = config.lines().count() + 1;
    assert_eq!(definition["range"]["start"]["line"], line, "{definition}");

    server.request("shutdown", Value::Null);
    server.notify("exit", Value::Null);
    assert!(server.child.wait().unwrap().success());
}
