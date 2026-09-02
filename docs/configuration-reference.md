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

## The bindings

They follow **Doom Emacs' non-evil scheme**. The classic Emacs keys are
untouched, and Doom's leader — `C-c` — carries the same maps:

| Prefix | What is under it |
|---|---|
| `C-c c` | Code: the language server, and `w` for trailing whitespace |
| `C-c f` | Files: `f` find, `d` dired, `D` delete, `m` move, `C` copy, `y`/`Y` copy the path, `p` this configuration |
| `C-c s` | Search: `p` the project, `b` this buffer, `i` the symbols |
| `C-c o` | Open: `t` terminal, `p` panel, `-` dired, `b` the desktop's viewer |
| `C-c t` | Toggle: `l` line numbers, `r` read-only, `c` fill column, `I` tabs or spaces, `w` wrapping |
| `C-c v` | Versioning: `g` magit, `/` its dispatch |
| `C-c m` | Cursors: `n`/`p`/`t`, and `<up>`/`<down>` for one per line |
| `C-c i` | Insert: `s` a snippet, `y` from the kill ring |
| `C-c q` | Quitting, and the session: `s` save, `l` restore |

`C-c l` and `C-c e` are left unbound on purpose: Doom uses them for the
localleader and for eval, and a global binding on either would take them away
from the modes that want them.

A `keymap "global"` block rebinds any of it.

---

## `init.rhai` — scripts

A file beside the configuration defining commands, in
[Rhai](https://rhai.rs). Each is registered with `define(name, doc, fn)` and
becomes an ordinary command: `M-x` offers it, a `keymap` block can bind it.

A command is a function taking one argument, a map describing where it was
called:

| Field | What it is |
|---|---|
| `text` | The whole buffer |
| `point` | The character offset point is at |
| `line`, `column` | Where that is, counted from zero |
| `buffer` | The buffer's name |
| `path` | The file it is visiting, or `()` |
| `mode` | Its major mode, or `()` |
| `region` | The selected text, or `()` |

and calling any of:

| Call | What it asks for |
|---|---|
| `insert(text)` | Put text in at point |
| `delete(count)` | Take characters out, forwards from point |
| `goto(offset)` | Move point |
| `run(command)` | Run one of the editor's own commands |
| `message(text)` | Say something in the echo area |
| `fail(text)` | Stop, and keep none of what was asked for |

A script cannot reach into editor state directly, and cannot take a built-in
command's name.

---

## `.editorconfig`

A file's own project speaks first. Any `.editorconfig` between the file and
the root is read, and what it says about that file overrides the settings
below for that buffer alone:

| Property | Overrides |
|---|---|
| `indent_style` | `indent-with-tabs` |
| `indent_size` / `tab_width` | `tab-width` |
| `end_of_line` | the buffer's line ending |
| `trim_trailing_whitespace` | `delete-trailing-whitespace` |
| `insert_final_newline` | `require-final-newline` |
| `max_line_length` | `fill-column` |

Nothing switches this on or off: a project either has an `.editorconfig` or
it does not.

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
| `truncate-lines` | bool | `#true` | Clip long lines rather than wrapping them, with a `$` in the last column of a line the edge cuts. Off, a long line carries on across the rows below it, breaking where the edge falls, with a `\` in the last column of each row that goes on; the line number is drawn once, against the first row. `C-c t w` toggles it. |
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
| `beacon` | bool | `#false` | Shine a light beside the cursor after it jumps, so the eye finds it again. The settings below are `beacon`'s own, under the same names, so a line copied from an Emacs configuration means what it meant there. |
| `beacon-size` | integer | `40` | Cells the light covers, counting the one point is on. |
| `beacon-blink-delay-ms` | integer | `300` | How long it stays at full length before it starts to fade. |
| `beacon-blink-duration-ms` | integer | `300` | How long the fade takes. It is eaten one cell at a time over this, so it shortens and dims together. |
| `beacon-color` | string | `"0.5"` | A number from 0 to 1 is a grey chosen against the background — light on a dark theme, dark on a light one — so one number is right either way. Anything else is read as a colour, such as `"#ff0066"`. |
| `beacon-blink-when-buffer-changes` | bool | `#true` | Light it when the buffer changes. |
| `beacon-blink-when-window-scrolls` | bool | `#true` | Light it when the window scrolls. |
| `beacon-blink-when-window-changes` | bool | `#true` | Light it when another window is selected. |
| `beacon-blink-when-point-moves-vertically` | integer | `0` | Lines point must move for a light; `0` never. Off by default because ordinary editing would light it constantly. |
| `session` | bool | `#false` | Remember what is open when the editor leaves, and open it again when it is next started in the same project with no file named. Kept under the state directory, keyed by the project's path. |
| `gui-font` | string | `"JetBrainsMono Nerd Font"` | The family the window draws with. Falls through a list of installed monospace families when it is not there. Only read when drawing into a window, which a `full` build does unless started with `-nw`. |
| `gui-font-size` | integer | `16` | Its size in pixels, clamped to 6–96. Physical pixels, so it is the same size on a display that reports a scale as on one that does not. |
| `autocomplete` | boolean | `#true` | Offer suggestions from the language server while typing, without being asked — `company-mode`. `C-M-i` asks for them whatever this says. `C-n`/`C-p` or the arrows move, `RET` or `TAB` takes one, `C-g` puts them away; every other key goes into the buffer and narrows the list. |
| `autocomplete-min-chars` | integer | `2` | How much of a word has to be typed before suggestions are offered for it, clamped to 1–10. One would offer them for every letter of every word. |
| `lsp-doc` | boolean | `#true` | Show what the language server knows about the symbol under point, in a box beside it, once the cursor has rested there — `lsp-ui-doc`. `C-c c k` asks for it whatever this says. |
| `which-key` | boolean | `#true` | After a pause in the middle of a key sequence, show what the next key can be. |
| `which-key-delay-ms` | integer | `400` | How long that pause is, capped at 10000. |
| `mouse-wheel-lines` | integer | `3` | How far one notch of the wheel moves the view, in lines, clamped to 1–50. A touchpad reports the pixels it moved and is unaffected. Only read when drawing into a window. |
| `smooth-scroll-ms` | integer | `300` | Roughly how long the view takes to come to rest after being asked to move, capped at 1000. Nine tenths of the way is covered in this long; the sliver after it is sub-pixel. Lower is brisker; `0` turns the animation off and the view arrives at once. A terminal cannot draw a fraction of a line and ignores it. |
| `scroll-animation-far-lines` | integer | `1` | How much of a jump the view animates when a command moves it a long way — a page, or `M->`. The last this many lines are drawn as a slide; the rest arrives at once. Capped at 8, `0` turns it off. Only read when drawing into a window. |
| `cursor-animation-ms` | integer | `150` | Roughly how long the cursor takes to slide to where point went, capped at 1000. `0` turns it off and the cursor is simply where it is. Only read when drawing into a window, which draws this instead of the beacon. |
| `cursor-short-animation-ms` | integer | `40` | How long a hop of a cell or two takes instead. Typing is mostly such hops, and giving them the full duration makes the cursor look like it is lagging behind the keyboard. Capped at 1000. |
| `cursor-trail` | integer | `70` | How far the back of the cursor lags the front while it travels, in percent — the smear. `0` moves the block rigidly. Capped at 95, because a back that never leaves is a cursor that never arrives. |
| `cursor-vfx` | string | `""` | What the cursor leaves behind it. `sonicboom`, `ripple` and `wireframe` mark where it landed; `railgun`, `torpedo` and `pixiedust` trail particles along the way it came. Empty is none of them, and nothing is drawn or computed. A name that is not one of these is reported rather than ignored. |
| `cursor-vfx-opacity` | integer | `78` | How solid the effect is at its strongest, in percent, capped at 100. |
| `cursor-vfx-particle-lifetime-ms` | integer | `500` | How long a trailing particle lives, capped at 5000. |
| `cursor-vfx-highlight-lifetime-ms` | integer | `200` | How long a mark at the destination takes to swell and fade, capped at 5000. |
| `cursor-vfx-particle-density` | integer | `70` | Particles per cell of distance travelled, in percent — `70` is seven for every ten cells. Capped at 2000, and a single jump never spawns more than 256 however high this is. |
| `cursor-vfx-particle-speed` | integer | `10` | How fast particles fly away, capped at 1000. |
| `cursor-vfx-particle-phase` | integer | `150` | How far round its arc a `railgun` flings them, in percent. Capped at 2000. |
| `cursor-vfx-particle-curl` | integer | `100` | How sharply a particle's flight turns as it goes, in percent. Capped at 2000. |
| `floating-blur` | boolean | `#true` | Blur what is behind a popup — the completion list, the doc box, the which-key panel, the doc box's own box. Costs the frame being drawn in two halves and three more passes over each popup's area, and nothing at all while none is open. Only read when drawing into a window. |
| `floating-blur-radius` | integer | `8` | How far the blur reaches, in pixels, capped at 64. `0` turns it off as surely as `floating-blur=#false`. |
| `floating-opacity` | integer | `82` | How solid a popup's own background is over the blur, in percent. Clamped to 20–100: a popup nobody can read is not a popup. Has nothing to show through unless the blur is on. |
| `ligatures` | boolean | `#true` | Let the font join characters it was made to draw as one, such as `!=`. A font with no such joins is unaffected. Only read when drawing into a window; a terminal draws with whatever font the terminal was given. |
| `shell` | string | `$SHELL` | The program a terminal tab starts. |

### Searching

| Option | Type | Default | Meaning |
|---|---|---|---|
| `case-fold-search` | bool or `#null` | `#null` | `#null` is smart case — case-insensitive unless the search string has an uppercase letter. `#true` and `#false` force it. |

### Language support

| Option | Type | Default | Meaning |
|---|---|---|---|
| `syntax-highlighting` | bool | `#true` | Use tree-sitter where a grammar is compiled in. |
| `grammar-auto-install` | bool | `#true` | Offer to fetch and build a grammar for a language that has none. Asks before doing anything; see [grammars.md](grammars.md). |
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
`M-x consult-theme` offer them. **`M-x consult-theme` is the one to reach for**: it
applies each theme as it comes under the cursor so you can see it, `RET` keeps
what is showing, and `C-g` puts back the one you started with. It changes
nothing on disk. **`M-x save-theme`** is what writes `set theme=` into your
configuration file — of whatever theme is in use, however it got there — and
`C-u M-x consult-theme` does both in one go. Writing it changes that one property
and nothing else in the file. Files are
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
`vertical-border`,
`line-number`, `line-number-current-line`, `mode-line`, `mode-line-inactive`,
`mode-line-buffer-id`, `minibuffer-prompt`, `echo-area`, `isearch`,
`isearch-fail`, `lazy-highlight`, `match-paren`, `trailing-whitespace`,
`fill-column-indicator`, `completion-selected`, `completion-annotation`,
`completion-border`, `completion-key`, `completion-count`, `which-key-group`,
`menu-heading`, `doc`, `doc-border`, `doc-title`, `doc-code`,
`terminal`,
`transient-key`, `transient-heading`, `transient-switch-on`,
`transient-switch-off`, `magit-section-heading`, `magit-section-highlight`,
`magit-diff-file-heading`,
`magit-diff-hunk-heading`, `magit-diff-added`, `magit-diff-removed`,
`magit-diff-context`, `magit-hash`, `magit-branch-local`,
`magit-branch-remote`, `magit-tag`,
`dired-header`, `dired-directory`, `dired-symlink`, `dired-marked`,
`dired-flagged`,
`terminal-tab`, `terminal-tab-selected`, `terminal-exited`, `panel-header`,
`panel-note`, `panel-current-buffer`, `symbol-detail`, `error`,
`warning`, `success`, and `default`, which the rest fall back to.

**Syntax** — `font-lock-keyword`, `font-lock-builtin`, `font-lock-constant`,
`font-lock-string`, `font-lock-comment`, `font-lock-doc`,
`font-lock-function-name`, `font-lock-variable-name`, `font-lock-type`,
`font-lock-property`, `font-lock-number`, `font-lock-operator`,
`font-lock-punctuation`, `font-lock-preprocessor`, `font-lock-escape`,
`font-lock-label`, `font-lock-attribute`, `font-lock-heading`,
`font-lock-link`.

**Diagnostics** — `diagnostic-error`, `diagnostic-warning`, `diagnostic-info`,
`diagnostic-hint`.

**File tree** — `tree-root`, `tree-directory`, `tree-file`, `tree-symlink`,
`tree-selected`, `tree-selection-mark`, `tree-arrow`, `tree-indent`,
`tree-git-modified`, `tree-git-added`,
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
