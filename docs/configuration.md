# Configuration language

*Why KDL, and what was rejected. For what each option **does**, see the
[configuration reference](configuration-reference.md); for a working file, see
[`config.example.kdl`](config.example.kdl); for whole themes, see
[`themes/`](themes).*

## Recommendation: KDL

`maxgus` configures itself in [KDL](https://kdl.dev) (`~/.config/maxgus/config.kdl`),
parsed with the `kdl` crate (v6, KDL 2.0).

### Why KDL over the alternatives

| Candidate | Verdict |
|---|---|
| **KDL** | Node-per-line with positional arguments and typed properties. Keymaps, faces and tree-view rules are all "a name plus a few arguments", which is exactly KDL's shape. One small pure-Rust dependency, no runtime. |
| TOML | Fine for flat settings, poor for keymaps: every binding becomes a table header or a quoted key, and nesting prefix maps reads badly. |
| A Lisp dialect | The idiomatic Emacs answer, and the wrong one here. An interpreter, a reader, a GC story and a stdlib is more code than the editor itself, against the "micro" goal. |
| Lua / Rhai | Real scripting, real weight — an embedded VM plus bindings for every editor API, and a startup cost we would then spend effort hiding. |
| RON | Serde-native and zero extra design work, but it reads as a serialised data structure rather than a config file. |

KDL keeps configuration declarative. The escape hatch is that every binding
names a **command** from the command registry, so anything the editor can do,
the config can bind — without the config file becoming a program.

### Shape

```kdl
// Settings are nodes with a single value.
set tab-width=4 indent-with-tabs=#false
set theme="maxgus-dark"
set line-numbers=#true

// Keymaps mirror Emacs notation directly.
keymap "global" {
    bind "C-x C-f" "find-file"
    bind "C-x C-s" "save-buffer"
    bind "M-x"     "execute-extended-command"
}

keymap "rust-mode" {
    bind "C-c C-c" "lsp-code-action"
}

// Faces override the active theme.
theme "maxgus-dark" {
    face "default"      fg="#c5c8c6" bg="#1d1f21"
    face "font-lock-keyword" fg="#b294bb" bold=#true
    face "region"       bg="#3b4a5c"
}

// Language servers.
lsp "rust" command="rust-analyzer"
lsp "python" command="pyright-langserver" args="--stdio"

// File tree.
tree {
    show-hidden #false
    ignore ".git" "target" "node_modules"
}
```

Unknown nodes are reported with a line number and skipped, so a config written
for a newer `maxgus` still loads on an older one.
