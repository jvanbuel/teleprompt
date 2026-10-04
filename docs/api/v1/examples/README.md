# Prompter API v1: examples

One message of each kind, exactly as the server sends or accepts it. They
are a contract: `crates/teleprompt-cli/tests/serve.rs` and `serve_voice.rs` check the server
against them, and the page is written against them. Change one
only with the API (`docs/design.md#prompter-api-version-1`).

| file | direction |
|---|---|
| `listening.json` | stdout of `teleprompt --format json serve`, once it listens |
| `script.json` | `GET /api/v1/script` |
| `voiced_script.json` | `GET /api/v1/script` for a project's script, as `serve --voice` serves it |
| `start.json`, `stop.json`, `discard.json`, `undo.json`, `keep_said.json`, `reword.json`, `instruct.json`, `cue.json`, `hold.json`, `move.json`, `stretch.json`, `undo_edit.json` | client to server, on the session socket |
| `reached.json`, `stopped.json`, `discarded.json`, `undone.json`, `kept_said.json`, `edited.json`, `edited_block.json`, `edit_undone.json`, `error.json` | server to client, on the session socket |
