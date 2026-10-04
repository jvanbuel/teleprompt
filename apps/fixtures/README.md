# App fixtures

`tour` is a two-line project the prompter's tests read to, the page's
(`crates/teleprompt-cli/tests/page`) and the Linux app's (`apps/linux/tests/ui.sh`):
its script is what `crates/teleprompt-listen-sherpa/tests/fixtures/two-lines.wav`
says. Its scene uses the mock scene plugin, so `teleprompt capture` makes its
clips without a browser or a terminal.
