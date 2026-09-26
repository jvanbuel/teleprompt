# Prompter API v1: examples

One message of each kind, exactly as the server sends or accepts it. They
are a contract: `crates/teleprompt-cli/tests/prompt.rs` checks the server
against them, and the macOS app's tests decode and encode them. Change one
only with the API (`docs/design.md#prompter-api-version-1`).

| file | direction |
|---|---|
| `listening.json` | stdout of `teleprompt --format json prompt`, once it listens |
| `script.json` | `GET /api/v1/script` |
| `start.json`, `stop.json` | client to server, on the session socket |
| `reached.json`, `stopped.json`, `error.json` | server to client, on the session socket |
