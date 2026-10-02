# App fixtures

`tour` is a two-line project the prompter apps' end-to-end tests read to:
its script is what `crates/teleprompt-listen-sherpa/tests/fixtures/two-lines.wav`
says. Its scene uses the mock adapter, so `teleprompt capture` makes its
clips without a browser or a terminal.

`setup-uses.json` is what `teleprompt --format json setup --uses` prints,
cut to three uses: what both apps' setup is tested against. Regenerate it
from the CLI when the report changes.
