<div align="center">

<h1>maxgus</h1>

<h3><em>A vibe-written lightweight editor inspired on emacs</em></h3>

<p>
Emacs keys, buffers and windows&nbsp; ·&nbsp; tree-sitter highlighting&nbsp; ·&nbsp; a language-server client<br>
themes you rewrite in a config file&nbsp; ·&nbsp; a treemacs-style file tree&nbsp; ·&nbsp; async throughout, on tokio
</p>

<p>
<a href="https://github.com/alejandro-llanes/maxgus-editor/actions/workflows/release.yml"><img alt="Release" src="https://img.shields.io/github/actions/workflow/status/alejandro-llanes/maxgus-editor/release.yml?style=for-the-badge&logo=githubactions&logoColor=white&label=release"></a>
<a href="https://github.com/alejandro-llanes/maxgus-editor/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/alejandro-llanes/maxgus-editor?style=for-the-badge&color=e05d44&display_name=tag&sort=semver"></a>
<a href="https://www.rust-lang.org"><img alt="Rust" src="https://img.shields.io/badge/rust-stable%20·%202024-000000?style=for-the-badge&logo=rust&logoColor=white"></a>
<a href="LICENSE"><img alt="Licence" src="https://img.shields.io/badge/licence-MIT-3d7ebb?style=for-the-badge"></a>
</p>

<p>
<img alt="Platforms" src="https://img.shields.io/badge/linux-333?style=flat-square&logo=linux&logoColor=white">
<img alt="" src="https://img.shields.io/badge/macos-333?style=flat-square&logo=apple&logoColor=white">
<img alt="" src="https://img.shields.io/badge/windows-333?style=flat-square&logo=windows&logoColor=white">
<img alt="" src="https://img.shields.io/badge/freebsd-333?style=flat-square&logo=freebsd&logoColor=white">
<img alt="Unsafe" src="https://img.shields.io/badge/unsafe-forbidden-4c9a2a?style=flat-square">
<img alt="Tests" src="https://img.shields.io/badge/tests-1435-4c9a2a?style=flat-square">
</p>

<sub><b>No Lisp interpreter. No plugin runtime.</b> ~37,000 lines · ten crates · one binary.</sub>

</div>

<br>

```console
$ maxgus src/main.rs
```

`C-h t` opens a guide, `C-h b` lists every binding, `C-x C-c` leaves.

---

## Install

Every [release](https://github.com/alejandro-llanes/maxgus-editor/releases/latest) carries a binary for
Linux (glibc, musl and aarch64), macOS (Intel and Apple Silicon), Windows and
FreeBSD, each with a `.sha256` beside it.

```console
$ curl -fsSL https://github.com/alejandro-llanes/maxgus-editor/releases/latest/download/maxgus-linux-x86_64.tar.gz | tar xz
$ ./maxgus-linux-x86_64/maxgus
```

<details>
<summary><b>The other builds</b></summary>

| Platform | Archive |
|---|---|
| Linux x86_64 (glibc) | `maxgus-linux-x86_64.tar.gz` |
| Linux x86_64 (static, musl) | `maxgus-linux-x86_64-musl.tar.gz` |
| Linux aarch64 | `maxgus-linux-aarch64.tar.gz` |
| macOS Intel | `maxgus-macos-x86_64.tar.gz` |
| macOS Apple Silicon | `maxgus-macos-aarch64.tar.gz` |
| Windows x86_64 | `maxgus-windows-x86_64.zip` |
| FreeBSD x86_64 | `maxgus-freebsd-x86_64.tar.gz` |

Each is `https://github.com/alejandro-llanes/maxgus-editor/releases/latest/download/<archive>`.

</details>

**Or build it** — Rust stable, edition 2024, no system dependencies beyond a
terminal:

```console
$ git clone https://github.com/alejandro-llanes/maxgus-editor
$ cd maxgus-editor && cargo build --release
$ ./target/release/maxgus
```

> [!NOTE]
> `C-z` job control is POSIX only. On Windows it reports that there is nothing
> to suspend into rather than pretending; everything else is the same editor.

---

## What it does

### Emacs keys, and they behave like Emacs

246 bindings across the `C-x`, `C-c`, `C-h`, `M-g` and `M-s` prefixes, driving
**234 commands**. Prefix arguments (`C-u`, `M-1`…`M-9`, `M--`), the mark and
the mark ring, the kill ring with `M-y`, registers, keyboard macros, rectangles,
narrowing, incremental and regexp search, `query-replace`, `occur`.

Not a lookalike: `C-k` at the end of a line takes the newline, the first `TAB`
grows to the common prefix and only the second shows the list, `M-a` never
moves forward, and undo groups the way Emacs groups it — one `C-/` undoes a run
of typing, not one character.

### Prompts that tell you what is there

`M-x` opens a bordered popup at the top of the frame. It lists every command
the moment it opens, with the key that runs each one and a line saying what it
does, and a count of where you are in the list:

```
╭──────────────────────────────────────────────────────────────────────────────╮
│2/11 M-x sbf                                                                  │
│save-buffer                   C-x C-s    Save this buffer to its file.        │
│save-buffer-anyway                       Save over a file that has changed on │
│set-buffer-file-coding-system C-x RET f  Choose the line endings this buffer i│
│save-buffers-kill-terminal    C-x C-c    Save and leave the editor.           │
│save-some-buffers             C-x s      Save every buffer with unsaved change│
│switch-to-buffer              C-x b      Display another buffer in this window│
╰──────────────────────────────────────────────────────────────────────────────╯
```

Matching is **fuzzy**: `sbf` finds `save-buffer`, `stb` finds
`switch-to-buffer`. Typing an uppercase letter makes the query case-sensitive.
`<up>` and `<down>` walk the list, `<prior>` and `<next>` move a page at a
time, and `RET` runs whatever is highlighted. `C-n` and `C-p` do the same as
the arrows for anyone who would rather not leave the home row.

`C-x b` gets the same popup for buffers, annotated with the file each one is
visiting. `TAB` still completes to the common prefix first, then cycles.

`C-x C-f` is the one prompt that answers with what you typed rather than what
it matched — otherwise you could never create `notes` next to a `notes-2024.md`
— but its list is right there, and `TAB` or the arrows still pick from it.

### Tree-sitter highlighting

Grammars for **Rust, Python, JavaScript, JSON, C, Bash, HTML and CSS** compiled
into the binary — nothing to install. Reparsing is incremental and only the
visible region is queried, so editing a 20,000-line file costs about 18 ms per
pause rather than a full parse. Every capture the grammars emit is checked
against a real face by a test, so nothing ships silently uncoloured.

### A language server client

Definitions, references, hover, completion, signature help, rename, formatting,
code actions, document and workspace symbols, and diagnostics in the buffer and
the mode line. Incremental document sync where the server asks for it. Server
requests are answered — including `workspace/applyEdit`, so a server can change
your text and be told whether it worked.

### A mode line worth looking at

State, buffer, position, branch and diagnostics, each in its own colour rather
than run together in punctuation:

```
  main.rs   12:4  34%    main   2   1
```

Nerd Font glyphs throughout, in the tree as well, chosen by file type — Rust,
Python, JSON, an image, an archive. `set nerd-font-icons=#false` turns them off
for terminals without such a font, and everything falls back to plain text.

### Themes you can rewrite without recompiling

Three built in (`maxgus-dark`, `maxgus-light`, `maxgus-term`) and **53 named
faces**, with `inherit`, bold/italic/underline/reverse/dim/strikethrough, and
truecolor degraded to 256 and then 16 colours by what your terminal reports.
`M-x load-theme` switches at runtime and keeps your overrides.

**`M-x visit-theme` tries them on.** Each theme is applied as it comes under
the cursor, so you choose by looking rather than by guessing what a name means.
`C-g` puts back the one you started with. Choose one and it asks whether to
write it into your config file or keep it for this session only — and writing
changes that one setting, leaving the rest of the file exactly as it was.

**A theme is a file you drop in.** Anything in
`~/.config/maxgus/themes/*.kdl` is picked up at startup; `set theme="nord"` is
all the configuration it takes. Four complete ones ship in
[`docs/themes/`](docs/themes) — Gruvbox, Nord, Dracula and Solarized Light —
each written entirely in configuration.

### A file tree modelled on treemacs

`C-x t t` opens it. **59 bindings and 44 commands** from treemacs' own keymap:
`n`/`p`, `M-n`/`M-p`, `u`, `TAB`, `RET`, `o v`/`o h`/`o r`/`o x`, `P` to peek,
`c f`/`c d` to create, `R`, `d`, `m`, `!`, `y a`/`y r`/`y p`/`y f` to copy
paths, `t h`/`t w`/`t f`/`t g`/`t d` to toggle, `g r` to refresh. Git status
in the gutter, follow mode, `?` for help.

### Asynchronous, and it means it

Every file read, directory walk, subprocess and language-server request runs on
tokio; parsing runs on tokio's blocking pool so a large file cannot stall the
runtime. Commands themselves are **synchronous** — a command is
`fn(&mut Editor, &Args) -> Result<()>` that queues work rather than awaiting —
which is why the whole command set is testable with no runtime, no terminal and
no filesystem.

A test walks the source and fails on a blocking call anywhere off the startup
path.

### It tries not to lose your work

- A file that is not text opens **read-only**, rather than being decoded, edited
  and written back with every unreadable byte replaced.
- A file **changed on disk** since it was read is not written over.
- `C-x C-w` will not quietly replace a file that already exists.
- Every destructive command refuses and names its override rather than asking
  for forgiveness.

---

## Keys worth knowing

The full list is `C-h b`, inside the editor. These are the ones worth knowing
first.

| Key | Does |
|---|---|
| `C-x C-f` | Visit a file |
| `C-x C-s` | Save |
| `C-x C-w` | Save under another name |
| `C-x C-c` | Leave |
| `C-x b` | Switch buffer — lists them as it opens |
| `C-x k` | Kill a buffer |
| `M-x` | Run a command — a fuzzy-matched popup of all of them |
| `<up>` `<down>` | In a prompt: walk the candidate list |
| `<prior>` `<next>` | In a prompt: a page of candidates at a time |
| `C-g` | Cancel |
| `C-x 2` `C-x 3` | Split below, split right |
| `C-x 0` `C-x 1` | Delete this window, delete the others |
| `C-x o` | Other window, cycling |
| `C-<left>` `C-<right>` | The window to the left, to the right |
| `C-<up>` `C-<down>` | The window above, below |
| `C-SPC` | Set the mark |
| `C-w` `M-w` | Kill the region, copy it |
| `C-y` `M-y` | Yank, then cycle back through the kill ring |
| `C-/` `C-M-/` | Undo, redo |
| `C-s` `C-r` | Incremental search forward, backward |
| `M-%` | Query replace |
| `M-g g` | Go to line |
| `M-;` | Comment or uncomment |
| `M-q` | Fill the paragraph |
| `M-h` `C-M-h` | Mark the paragraph, the definition |
| `C-x z` | Repeat the last command |
| `C-x t t` | Toggle the file tree |
| `M-.` `M-,` | Go to definition, come back |
| `C-c l r` | Rename through the language server |
| `C-c l f` | Format the buffer |
| `C-h b` | Every binding |
| `C-h t` | The tutorial |
| `C-h v` | A setting's current value |

Inside the tree, treemacs' own keys apply: `n` and `p` to move, `u` for the
parent, `TAB` to fold, `RET` to open, `c f` and `c d` to create, `R` to rename,
`d` to delete, `y a` to copy the path, `?` for the rest.

---

## Configuring it

`~/.config/maxgus/config.kdl`, in [KDL](https://kdl.dev):

```kdl
set tab-width=4 theme="maxgus-dark" line-numbers=#true

keymap "global" {
    bind "C-c f" "lsp-format-buffer"
    unbind "C-z"
}

theme "maxgus-dark" {
    face "font-lock-comment" fg="#5f6b73" italic=#true
}

lsp "rust" command="rust-analyzer" {
    root-markers "Cargo.toml"
}

tree { width 32; ignore ".git" "target" }
```

A KDL node is "a name, some arguments, some properties, optionally a block" —
which is exactly the shape of a keybinding, a face, a server and an ignore rule.
One small pure-Rust dependency, no VM, no startup cost. Every binding names a
**command from the registry**, so configuration reaches anything the editor can
do without becoming a program.

| | |
|---|---|
| [**Configuration reference**](docs/configuration-reference.md) | Every option, its type and its default |
| [Why KDL](docs/configuration.md) | And what was rejected: TOML, a Lisp dialect, Lua, RON |
| [Worked example](docs/config.example.kdl) | Every option in one annotated file |
| [Example themes](docs/themes) | Gruvbox, Nord, Dracula, Solarized Light |

Unknown settings get a "did you mean"; unknown faces are reported with the line
they are on. Documentation that has drifted is a bug here — tests assert the
example config exercises every setting and every face attribute the parser
accepts.

---

## Layout

Ten crates, `unsafe_code = "forbid"` across all of them.

| Crate | What it holds |
|---|---|
| `maxgus-text` | Rope buffers, point and mark, grouped undo, kill ring, registers, motions, search |
| `maxgus-keys` | Emacs key notation, keymaps, global/mode/minor precedence |
| `maxgus-config` | The KDL parser |
| `maxgus-faces` | Colours, faces, themes, colour degradation |
| `maxgus-syntax` | Tree-sitter grammars and incremental highlighting |
| `maxgus-lsp` | JSON-RPC framing, position encoding, the client |
| `maxgus-tree` | File tree model, git status, the treemacs keymap |
| `maxgus-tui` | Cell grid, frame diffing, terminal setup, job control |
| `maxgus-core` | Buffers, windows, minibuffer, commands, dispatch, redisplay |
| `maxgus` | The event loop and the task executor |

## Testing

**1435 tests.** Unit tests beside the code; session tests that press real keys
through the real keymap and assert on the rendered screen; smoke tests that open
a pseudo-terminal, run the built binary and read what it draws — including
against a real `clangd`.

Plus a reproducible pseudo-random walk over the entire keymap that presses
bindings, answers prompts, redraws and checks invariants after every step. It
prints the seed and the key sequence when it fails, which reproduces exactly.

The habit that has found the most: **sabotage the code and check the test
fails.** A test that has never been observed to fail is a hypothesis, not a
check.

## Releasing

Tagging is the whole of it:

```console
$ git tag v0.1.0 && git push origin v0.1.0
```

[`.github/workflows/release.yml`](.github/workflows/release.yml) builds all
seven targets — three Linux, two macOS, Windows and a cross-compiled FreeBSD —
packages each with its checksum, and attaches them to the release.

---

<div align="center">

**MIT** · built in the open at
[github.com/alejandro-llanes/maxgus-editor](https://github.com/alejandro-llanes/maxgus-editor)

</div>
