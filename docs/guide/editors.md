# Editors

`teleprompt lsp` is a language server: your editor starts it, sends it the
scripts you open, and shows what it says as you type, before you save.

- **Problems where they are.** Everything `teleprompt check` reports, on
  the line it is about: an unknown speaker or scene, an attribute that
  isn't one, a policy that doesn't parse, an include that isn't there,
  and the compile's warnings.
- **Completion.** In a line's braces, its attributes and, after `@`, the
  cast. On a ` ```teleprompt ` fence, a block's attributes, and the values
  of `scene=`, `policy=`, `align=` and `include=`.
- **Hover.** On a line, who says it, with which voice, how long it takes
  and when it starts. On a block, what it plays. On `@guest` or
  `scene=terminal`, what that is.
- **Go to definition.** From `@guest` to its `[voices.guest]` table, from
  `scene=` to the scene's configuration, and from `include=` to the file.
- **Outline.** Chapters, and the lines and blocks in each.

The server reads the project the script is in, `teleprompt.toml` and the
script's front matter, the way `check` does. It synthesizes nothing and
calls no backend, so it costs nothing to leave running.

Editors send it every Markdown file. It only speaks up about scripts: a
file whose front matter has `teleprompt:`, or that has a ` ```teleprompt `
block. The rest of your Markdown is left alone, so it can sit beside
another Markdown server.

## Neovim

Neovim 0.11 and later:

```lua
vim.lsp.config("teleprompt", {
  cmd = { "teleprompt", "lsp" },
  filetypes = { "markdown" },
  root_markers = { "teleprompt.toml", ".git" },
})
vim.lsp.enable("teleprompt")
```

## Helix

In `~/.config/helix/languages.toml`, alongside whatever Markdown server
you already use:

```toml
[language-server.teleprompt]
command = "teleprompt"
args = ["lsp"]

[[language]]
name = "markdown"
language-servers = ["marksman", "teleprompt"]
```

## Emacs

With eglot:

```elisp
(with-eval-after-load 'eglot
  (add-to-list 'eglot-server-programs
               '(markdown-mode . ("teleprompt" "lsp"))))
```

Then `M-x eglot` in a script.

## Others

Any editor with a generic language server client can run it: the command
is `teleprompt lsp`, it speaks on stdin and stdout, and it wants whole
documents on each change. VS Code needs an extension to start a server,
and there isn't one yet.
