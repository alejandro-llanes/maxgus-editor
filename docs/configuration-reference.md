# Configuration reference

Every option `maxgus` understands, with its type and default.

The file lives at `~/.config/maxgus/config.kdl`. Start elsewhere with
`--config <file>`, or with nothing at all using `-Q`. From inside the editor,
`C-c e` (`M-x edit-configuration`) opens whichever file this session was
started with, creating it if it is not there yet. Every node is optional.
A node or key `maxgus` does not recognise is reported with its line number and
skipped, so a file written for a newer version still starts an older one — and
a misspelling gets a "did you mean" rather than silence.

See [configuration.md](configuration.md) for why KDL, and
[config.example.kdl](config.example.kdl) for a working file that exercises
every option below.

---

## `set` — settings

Written as properties on a `set` node. Several to a line, or one per line:

```kdl
set tab-width=4 indent-with-tabs=#false
set theme="maxgus-dark"
```

KDL spells booleans `#true` and `#false`, and null `#null`.

### Editing

| Option | Type | Default | Meaning |
|---|---|---|---|
| `tab-width` | integer ≥ 1 | `4` | Columns a tab occupies, and the width of one indentation level. |
| `indent-with-tabs` | bool | `#false` | Indent with tab characters instead of spaces. |
| `fill-column` | integer | `70` | Column `M-q` fills to. |
| `fill-column-indicator` | bool | `#false` | Draw a rule at the fill column. |
| `require-final-newline` | bool | `#true` | Ensure the file ends with a newline when saving. |
| `delete-trailing-whitespace` | bool | `#false` | Strip trailing spaces from every line on save. |
| `backup-files` | bool | `#false` | Copy `file` to `file~` before overwriting it. |
| `kill-ring-max` | integer ≥ 1 | `120` | Entries kept for `C-y` and `M-y`. |

### Display

| Option | Type | Default | Meaning |
|---|---|---|---|
| `theme` | string | `"maxgus-dark"` | Active theme: a built-in name, or one a `theme` block defines. |
| `line-numbers` | bool | `#false` | Show a line-number column. |
| `truncate-lines` | bool | `#true` | Clip long lines rather than wrapping them. |
| `scroll-margin` | integer | `0` | Lines of context kept above and below point. |
| `blink-cursor` | bool | `#false` | Ask the terminal for a blinking block cursor. |
| `nerd-font-icons` | bool | `#true` | Glyphs in the file tree and mode line, chosen by file type. Needs a [Nerd Font](https://www.nerdfonts.com); turn it off and both fall back to plain text. |
| `echo-keystrokes-ms` | integer | `1000` | Pause before a half-typed `C-x …` is echoed. |
| `panel-tree` | bool | `#true` | Show the file tree section of the side panel. |
| `panel-symbols` | bool | `#true` | Show the symbol outline section. It hides itself anyway when no language server is running for the buffer being edited. |
| `panel-buffers` | bool | `#true` | Show the open-buffers section. |
| `panel-at-startup` | bool | `#false` | Open the side panel as soon as the editor starts. |
| `panel-symbols-height` | integer | `12` | Rows the symbol outline's window takes. The tree takes whatever the others leave. |
| `panel-buffers-height` | integer | `8` | Rows the buffer list's window takes. |
| `gui-font` | string | `"JetBrainsMono Nerd Font"` | The family the window draws with. Falls through a list of installed monospace families when it is not there. Only read by a `--features gui` build started with `--gui`. |
| `gui-font-size` | integer | `16` | Its size in pixels, clamped to 6–96. |
| `shell` | string | `$SHELL` | The program a terminal tab starts. |

### Searching

| Option | Type | Default | Meaning |
|---|---|---|---|
| `case-fold-search` | bool or `#null` | `#null` | `#null` is smart case — case-insensitive unless the search string has an uppercase letter. `#true` and `#false` force it. |

### Language support

| Option | Type | Default | Meaning |
|---|---|---|---|
| `syntax-highlighting` | bool | `#true` | Use tree-sitter where a grammar is compiled in. |
| `lsp-enabled` | bool | `#true` | Start a language server for buffers whose language has one configured. |
| `idle-delay-ms` | integer | `150` | Quiet time before re-highlighting and syncing with the server. |

---

## `keymap` — key bindings

A block named `"global"`, or after a major mode (`"rust-mode"`, `"python-mode"`,
…) for bindings that apply only in that mode. Repeated blocks with the same
name merge.

The file tree has a mode of its own, `"treefile-mode"`, holding the treemacs
keymap. A block of that name **adds to** the built-in bindings rather than
replacing them, so rebinding one key does not cost you the other fifty-eight.

```kdl
keymap "global" {
    bind "C-c f" "lsp-format-buffer"
    unbind "C-z"
}

keymap "rust-mode" {
    bind "C-c C-c" "lsp-code-action"
}
```

| Node | Arguments | Meaning |
|---|---|---|
| `bind` | key sequence, command name | Bind a sequence to a command. |
| `unbind` | one or more key sequences | Remove bindings entirely. |

**Key notation** is Emacs': `C-` control, `M-` meta, `S-` shift. Multi-key
sequences are separated by spaces — `"C-x r SPC"`. The named keys, with the
alternative spellings each accepts:

| Key | Also written |
|---|---|
| `SPC` | `<space>` |
| `RET` | `<return>`, `<enter>` |
| `TAB` | `<tab>` |
| `DEL` | `<backspace>` |
| `ESC` | `<escape>` |
| `<delete>` | `<deletechar>` |
| `<prior>` | `<pageup>` |
| `<next>` | `<pagedown>` |
| `<up>` `<down>` `<left>` `<right>` | |
| `<home>` `<end>` `<insert>` | |
| `<f1>`, `<f2>`, … | any `<fN>` |

**Command names** are the ones `M-x` lists and `C-h b` prints. A binding naming
a command that does not exist is refused at startup rather than becoming a dead
key.

---

## `theme` — faces

A block naming a theme. Only the differences from the base need stating;
everything else is inherited.

```kdl
// Adjusting a built-in: the block is named after it.
theme "maxgus-dark" {
    face "font-lock-comment" fg="#5f6b73" italic=#true
    face "region"            bg="#3a4048"
}

// A theme of your own says which built-in it starts from.
theme "midnight" base="maxgus-dark" {
    face "region" bg="#001133"
}
```

| Property | Meaning |
|---|---|
| `base=` | The built-in this theme starts from, so anything it leaves unset still has a value. Omitted, it starts from the built-in of the same name, and failing that the default. **A light theme must set `base="maxgus-light"`**, or every face it does not mention comes out dark. |

### Themes as files

Anything in `~/.config/maxgus/themes/*.kdl` is read at startup, so a theme is a
file you drop in rather than something to paste into `config.kdl`:

```
~/.config/maxgus/
├── config.kdl          set theme="nord"
└── themes/
    ├── nord.kdl        theme "nord" base="maxgus-dark" { … }
    └── gruvbox.kdl     theme "gruvbox" base="maxgus-dark" { … }
```

`set theme=` finds them by name, and both `M-x load-theme` and
`M-x visit-theme` offer them. **`M-x visit-theme` is the one to reach for**: it
applies each theme as it comes under the cursor so you can see it, `C-g` puts
back the one you started with, and choosing one asks whether to write
`set theme=` into your configuration file or keep it for the session only.
Writing it changes that one property and nothing else in the file. Files are
read in name order, and a `theme` block in `config.kdl` still wins face by face
over a file of the same name, so you can adjust a downloaded theme without
editing it. A file that will not parse, or that names a face that does not
exist, is reported at startup and skipped rather than stopping the editor.

Four complete examples are in [`themes/`](themes), ready to copy.

### Face attributes

| Attribute | Type | Meaning |
|---|---|---|
| `fg`, `foreground` | string | Foreground colour. |
| `bg`, `background` | string | Background colour. |
| `inherit` | string | Copy whatever the named face sets, for anything left unset here. |
| `bold` | bool | |
| `italic` | bool | |
| `underline` | bool | |
| `reverse` | bool | Swap foreground and background. |
| `dim` | bool | |
| `strikethrough` | bool | |

Attributes a terminal does not support are simply ignored by it.

### Colours

Written as a string, in any of these forms:

| Form | Example |
|---|---|
| Six-digit hex | `"#5f6b73"` |
| Three-digit hex | `"#abc"` |
| ANSI palette index, `0`–`255` | `"12"` |
| Colour name | `"red"`, `"bright-blue"` |
| The terminal's own | `"default"`, `"none"` |

The sixteen names are `black`, `red`, `green`, `yellow`, `blue`, `magenta`,
`cyan` and `white`, each also with a `bright-` prefix. Truecolor is degraded to
256 and then to 16 colours according to what `TERM` and `COLORTERM` report, so
a hex colour still shows sensibly on a limited terminal.

### Built-in themes

`maxgus-dark`, `maxgus-light`, and `maxgus-term` — which names only the sixteen
ANSI colours, so it follows whatever palette the terminal is set to.

`M-x load-theme` switches between them without a restart, keeping the faces
your `theme` blocks override.

### Face names

**Interface** — `cursor`, `region`, `highlight`, `shadow`, `fringe`,
`line-number`, `line-number-current-line`, `mode-line`, `mode-line-inactive`,
`mode-line-buffer-id`, `minibuffer-prompt`, `echo-area`, `isearch`,
`isearch-fail`, `lazy-highlight`, `match-paren`, `trailing-whitespace`,
`fill-column-indicator`, `completion-selected`, `completion-annotation`,
`completion-border`, `completion-key`, `completion-count`, `terminal`,
`transient-key`, `transient-heading`, `transient-switch-on`,
`transient-switch-off`, `magit-section-heading`, `magit-section-highlight`,
`magit-diff-file-heading`,
`magit-diff-hunk-heading`, `magit-diff-added`, `magit-diff-removed`,
`magit-diff-context`, `magit-hash`, `magit-branch-local`,
`magit-branch-remote`, `magit-tag`,
`terminal-tab`, `terminal-tab-selected`, `terminal-exited`, `panel-header`,
`panel-note`, `panel-current-buffer`, `symbol-detail`, `error`,
`warning`, `success`, and `default`, which the rest fall back to.

**Syntax** — `font-lock-keyword`, `font-lock-builtin`, `font-lock-constant`,
`font-lock-string`, `font-lock-comment`, `font-lock-doc`,
`font-lock-function-name`, `font-lock-variable-name`, `font-lock-type`,
`font-lock-property`, `font-lock-number`, `font-lock-operator`,
`font-lock-punctuation`, `font-lock-preprocessor`, `font-lock-escape`,
`font-lock-label`, `font-lock-attribute`.

**Diagnostics** — `diagnostic-error`, `diagnostic-warning`, `diagnostic-info`,
`diagnostic-hint`.

**File tree** — `tree-root`, `tree-directory`, `tree-file`, `tree-symlink`,
`tree-selected`, `tree-arrow`, `tree-git-modified`, `tree-git-added`,
`tree-git-deleted`, `tree-git-untracked`, `tree-git-ignored`,
`tree-git-conflict`.

A `face` naming something not on this list is reported at startup with a "did
you mean", because it would otherwise never paint and never say why.

---

## `lsp` — language servers

One node per language, named by the same identifier the editor derives from the
file extension.

```kdl
lsp "rust" command="rust-analyzer" {
    root-markers "Cargo.toml"
}

// One argument fits on the property; several go in an `args` node.
lsp "typescript" command="typescript-language-server" {
    args "--stdio" "--log-level" "2"
    root-markers "tsconfig.json" "package.json"
}
```

| Key | Where | Meaning |
|---|---|---|
| `command=` | property, required | The executable to run. |
| `args=` | property | A single argument. |
| `args` | child node | Any number of arguments. |
| `root-markers` | child node | Files whose presence marks the project root. The first found walking upwards wins; failing all of them, the directory the editor was started in. |

Language identifiers, as derived from the file name: `rust`, `python`,
`javascript`, `typescript`, `json`, `c`, `cpp`, `bash`, `html`, `css`, `toml`,
`markdown`, `kdl`, `go`, `yaml`, and `make` and `dockerfile` for `Makefile` and
`Dockerfile`. Of these, `rust`, `python`, `javascript`, `json`, `c`, `bash`,
`html` and `css` also have a tree-sitter grammar compiled in; the rest are
recognised for the mode line and for choosing a language server, and are shown
without syntax colouring.

---

## `tree` — the file tree

```kdl
tree {
    width 32
    show-hidden #false
    directories-first #true
    git-status #true
    follow #true
    ignore ".git" "target" "node_modules"
}
```

| Node | Type | Default | Meaning |
|---|---|---|---|
| `width` | integer | `32` | Width of the side window in columns. |
| `show-hidden` | bool | `#false` | Show dotfiles. |
| `directories-first` | bool | `#true` | Sort directories above files. |
| `git-status` | bool | `#true` | Show the git status column. |
| `follow` | bool | `#true` | Keep the selection on the file being edited. |
| `ignore` | strings | `target`, `node_modules`, `.git` | Names never shown. Replaces the default list rather than adding to it, so list everything you want ignored. |

The five boolean nodes may be written bare, which reads as on: `show-hidden`
means `show-hidden #true`.

---

## Where settings are visible from inside the editor

- `C-h v` — a setting's current value.
- `C-h b` — every key binding in effect.
- `C-h m` — the current mode and its bindings.
- `M-x` — every command, listed as the prompt opens and narrowed as you type.
