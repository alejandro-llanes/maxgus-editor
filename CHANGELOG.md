# Changelog

## Unreleased

- **Suggestions while typing.** `company-mode`, for a language server: after
  a couple of letters the server is asked what could follow, and a list
  appears at the cursor with each candidate's kind and type. Typing narrows
  it — fuzzily, the way `M-x` does — `C-n`/`C-p` move, `RET` or `TAB` takes
  one, `C-g` puts it away, and every other key goes into the buffer.

  An idle pause never inserts anything on its own; `C-M-i` on a single
  candidate still completes it outright, the way it always did.
  `set autocomplete=#false`, `set autocomplete-min-chars=`.
- **The doc box reads like a document.** Hover replies are markdown, and
  were drawn as the markdown they are — `### function \`add\`` and a row of
  hyphens, which is worse than plain prose because the punctuation is in the
  way too. Headings are bold now, `---` is a rule across the box, `- ` is a
  bullet, and code — inline or fenced — sits on a panel of its own, themeable
  as `doc-code`.
- **Markdown is asked for.** `contentFormat` listed `plaintext` first, and
  that list is a preference order servers honour: clangd was sending a wall
  of text with the structure flattened out of it. Markdown first is what
  gives the box something to format.
- The box is half the window at most rather than a third, and the "… N more
  lines" notice clears the row it replaces rather than leaving the tail of
  the line beneath it showing.

## v0.2.1

Grammars from the system, and a different eleven built in.

### Grammars the editor was not built with

- **Point it at a directory and it loads them.** `libtree-sitter-<language>.so`
  is how every package manager ships a tree-sitter grammar, and how Neovim
  and Helix consume them; maxgus now does too. Eleven languages are built
  in and every other one was uncoloured — now it need not be.

  ```kdl
  grammars {
      search "/usr/lib"
      queries "/usr/share/tree-sitter/queries"
  }
  ```

  Nothing is loaded unless the configuration says where to look. There are
  no default directories.

- **`M-x describe-grammars`** says what is built in, what loaded, what would
  not and why, and every directory searched.
- **[docs/grammars.md](docs/grammars.md)** has per-platform instructions —
  Arch, Debian, Fedora, Homebrew, Nix, Windows — how to build one yourself,
  what each error means, and what loading a shared library into an editor
  means for trust.
- `maxgus-syntax` is the one crate permitted to write `unsafe`, at `deny`
  rather than `forbid`. `dlopen`, `dlsym` and `LanguageFn::from_raw` have no
  safe form, and `src/dynamic.rs` says at length what each assumes and what
  is checked instead.

### Built-in grammars

- **Now**: c, html, ini, javascript, json, markdown, python, rust, toml,
  xml, yaml. **Gone**: bash and css — which is why the binary is *smaller*
  than v0.2.0's, not larger.
- KDL and Rhai were meant to be here and are not. `tree-sitter-kdl` is bound
  to tree-sitter 0.20, whose C runtime will not link beside 0.26's, and it
  is a KDL **v1** grammar besides — maxgus reads v2. Rhai has no published
  crate. `docs/grammars.md` says so rather than recommending a grammar that
  half-parses your configuration.
- `font-lock-heading` and `font-lock-link` are new faces, because markdown
  and XML would otherwise have rendered almost colourless.

### Also

- An extension nothing knows now names its own language — `main.zig` is
  `zig` — which is what makes a grammar findable without a table of every
  language there has ever been.
- An LSP request for a language with no server used to leave "Language
  server: describing..." on screen for ever. It says so now, but only when a
  command announced it: the symbols panel and the doc box ask while a server
  may still be starting.

## v0.2.0

Three builds to pick from, a window that works, and one line to install any
of them.

### Three builds, and a way to get them

- **`minimal`, `full` and `gui`** are now what a release carries, for every
  platform that can build them — `minimal` and `full` everywhere, `gui`
  where a window system is there to compile against. Archives are named
  `maxgus-<build>-<platform>`, each with a `.sha256` beside it.
- **One line installs any of them**, from a static page that publishes
  itself:

  ```sh
  curl -fsSL https://alejandro-llanes.github.io/maxgus-editor/install.sh | sh
  ```

  It works out the platform, checks the download against the published
  checksum, and refuses to install anything that does not match. `--build`,
  `--version`, `--prefix`, `--dry-run`.

### New

- **which-key.** Pause in the middle of `C-x` or `C-c` and a panel says what
  the next key can be, with keys that open another map shown as the group
  they open. In all three builds. `set which-key=#false`,
  `set which-key-delay-ms=400`.
- **lsp-ui-doc.** Rest the cursor on a symbol and what the language server
  knows about it appears in a box beside the line, on whichever side has
  room. Was a help window over the code. `set lsp-doc=#false`;
  `C-c c k` asks for it either way.
- **A window that opens by itself.** A `gui` build is a desktop program:
  `maxgus` opens a window, `maxgus -nw` takes the terminal — the spelling
  Emacs has used for thirty years, accepted by every build. With no display
  to draw into it starts in the terminal rather than failing.
- **The wheel is adjustable**: `mouse-wheel-lines` is how far a notch goes,
  `smooth-scroll-ms` how long the slide lasts (`0` for none). The ease is
  now measured in real time, so it is the same speed at 60Hz and 144Hz.

### Fixed

- **The window crashed** as soon as a frame needed more quads than the last
  allocation — the instance buffer recorded a capacity it had not allocated.
- **The theme did nothing in the window.** Colours were being gamma-encoded
  twice, so `#1d1f21` reached the screen as `#5e6164`: a dark theme rendered
  as flat grey. `load-theme` now reaches the window too.
- **Scrolling shook the whole editor.** The sub-line offset was applied to
  every row, so a wheel notch slid the mode line, the echo area and the file
  tree with the text. Only the scrolling window's text moves now, and the
  line arriving is drawn into the gap that opens.
- **The window did none of the per-turn work**: no macro replayed, the file
  tree never followed the file being edited, the buffer was never
  re-highlighted after a change, and the language server was never told
  anything had changed. Both front ends share one copy of it now.
- **The window never slept**, costing a sixth of a core to show a still
  screen. Idle is 0% now.
- **The close button threw away unsaved work.** It runs the same command
  `C-x C-c` does, and refuses.
- **Expanding a directory sent the file tree's cursor to the root.**
- The window's title names the buffer and marks it while it is modified; the
  wheel scrolls the window under the pointer; the font is sized in physical
  pixels, so it is not half-size on a display that reports a scale.

### Also

- 1992 tests, up from 1650.
- `docs/configuration-reference.md` is checked against the setting list, so
  it cannot fall behind the parser the way it had.

## v0.1.0

The first release. Emacs keys, buffers and windows, tree-sitter
highlighting, a language-server client, magit, a treemacs-style file tree,
themes in a configuration file, and a terminal panel.
