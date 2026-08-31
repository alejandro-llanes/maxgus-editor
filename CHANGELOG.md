# Changelog

## Unreleased

- **Scrolling and the cursor move on a spring now, not an easing curve.**
  This is the one that was felt before it was found. An exponential ease —
  a fixed fraction of the remaining distance each frame — is *fastest at its
  very first frame* and slower every frame after, which the eye reads as a
  snap followed by a crawl, and the crawl is the part it notices. A
  critically damped spring starts at rest, accelerates and settles without
  overshooting, which is how something being moved actually moves.

  It also carries a velocity, so a second wheel notch while the first is
  still arriving adds to it instead of starting it over. Spinning a wheel
  builds up rather than stuttering.

  What the duration setting means changed with it, and honestly: nine tenths
  of the way is covered in the time it names — the same nine tenths whatever
  the distance and whatever the duration — and the sliver after that happens
  below a quarter of a pixel. The tests say that rather than claiming an
  arrival time that was never true. The defaults moved with it:
  `smooth-scroll-ms` is 300 and `cursor-animation-ms` 150.

- **Typing does not smear.** A hop of a cell or two now gets
  `cursor-short-animation-ms` — 40ms against 150 — because animating a
  keystroke over the duration meant for crossing the screen makes the cursor
  look like it is lagging behind the keyboard. It is most of what a cursor
  ever does, and it was the other half of why this felt worse than the thing
  it was copied from.

- **A slide no longer redraws the whole frame every frame.** The lines that
  fill the gap it opens are fetched by drawing the frame again into a
  scratch surface, and that was being done *per frame for the length of
  every scroll* — a second complete redisplay, sixty times a second, of a
  screen that had not changed. They are fetched once now and kept until the
  view moves, the buffer changes or a key is pressed.

- **Six things for the cursor to leave behind.** `sonicboom`, `ripple` and
  `wireframe` mark where it landed with a disc, a ring or a square that
  swells and fades; `railgun`, `torpedo` and
  `pixiedust` trail particles along the way it came, each with its own
  flight, rotation and lifetime. Off unless `cursor-vfx` names one, and then
  eight more settings for tuning it. The renderer grew a disc primitive with
  its edge worked out in the shader, so a sonic boom stays smooth however
  large it gets.

- **What is behind a popup is blurred.** The completion list, the doc box,
  the which-key panel, the tree's `?` panel: each sits on a blurred copy of
  what it covers rather than on a hole cut in the text.

  Which needed the frame split in two. Redisplay composited the popups into
  the same grid as everything else, so by the time a front end saw the cells
  there was nothing behind them to blur — `draw` is now `draw_background`
  and `draw_floating`, the second returning where it put things. Drawing the
  two halves costs what drawing them together cost; the copy between them is
  what the blur is bought with. The backdrop is only drawn near the popups,
  because a blur reaches no further than its radius, and none of it happens
  at all while no popup is open. `floating-blur`, `floating-blur-radius`,
  `floating-opacity`.

- **Ligatures, in the window.** `!=` drawn as the one mark the font's
  designer drew, and `->`, `=>`, `<=`, `>=`, `|>`, `...` with it. The text is
  *shaped* now — handed to a shaper a run at a time — rather than looked up a
  character at a time, so which characters join is the font's answer and not
  a list kept here: a font that joins nothing is simply unaffected.

  Glyphs are keyed by the font's own index rather than by character, because
  a ligature is a glyph no character names. Runs stop at a space and at a
  change of style, since a mark half in bold is two fonts pretending to be
  one glyph. `set ligatures=#false` turns it off.

  Worth knowing how a monospace coding font does this, because it is not what
  it sounds like: it does *not* collapse two cells into one glyph — that
  would cost it a column. It substitutes both cells with halves of the joined
  mark and keeps the count. So `!=` is still two glyphs; they are simply not
  the `!` and the `=` those characters draw alone.

- **An animated cursor.** The block slides to where point went instead of
  appearing there, and smears on the way: the four corners are animated
  separately and the ones at the back are given less of the distance, so it
  stretches out behind itself while it travels and gathers into a cell when
  it lands. That is what lets the eye follow it across a long jump rather
  than having to find it again. `cursor-animation-ms` is how long arriving
  takes — the setting names the *slowest* corner, because that is the one
  still arriving — and `cursor-trail` is how far the back lags, in percent.

  The window draws this **instead of the beacon**. They answer the same
  question, and the beacon only exists because a terminal cannot show the
  journey; both at once is the answer given twice with the eye pulled two
  ways. `set cursor-animation-ms=0` gives the beacon back.

  The renderer grew a third pipeline for it. A smear is not upright, and a
  shape with no width and height cannot be given as a position and a size.

- **The view a command moves slides too.** Smooth scrolling was the wheel's
  alone: `C-v`, a search landing off the screen and `M->` all teleported. The
  wheel and a command move the view from opposite ends — the wheel asks and
  the editor follows, a command has already gone before anything is drawn —
  so this is the other direction, the drawing starting where the view was and
  catching up. It never turns into a line of its own, because the line has
  already been crossed.

  A long jump is not slid in full; the last `scroll-animation-far-lines` of
  it are, which is the part that says which way it went. The gap that opens
  is filled with the lines arriving, which took one redraw however deep the
  gap is — asking per line would have made a four-line slide cost four
  screens a frame.

- **`?` in the file tree shows the whole keymap at once.** It opened a
  fifty-line `*Help*` buffer in the window beside the tree — which is to
  say it took the file being edited off the screen to tell you which key
  moves down one line. treemacs does not do that: its `?` summons a hydra,
  and this is that hydra, drawn in the box `C-x` and `C-c` already draw
  into. Eleven named columns — Navigation, Nodes, Opening, Files, Copying,
  Root, Toggles, Sections, Width, Refreshing, Leaving — one row per command
  with the key that reaches it soonest, and short phrases rather than
  command names, because these are columns and a sentence in a column is a
  sentence that gets cut.

  **The keys stay live underneath it**, which is the point of it and what
  treemacs' `:exit nil` does: the tree can be walked with the map still up,
  so reading what `n` does and pressing it does not take two goes. `C-g`
  puts it away, pressing `?` again puts it away, and leaving the tree puts
  it away. Sections are kept whole and the shortest arrangement that fits
  is the one drawn, so a wide window gets a short panel rather than a tall
  one with the last three sections missing. What genuinely will not fit is
  counted. In all three builds.

  A test holds the panel and the keymap together in both directions: a
  binding the help never mentions, and a key the help teaches that the map
  does not have, both fail it.

- **The doc box is a panel rather than a hole cut in the buffer.** It was
  the buffer's own background with a grey line around it — the same line
  every popup gets — so a reply from the language server read as a
  rectangle of the same text rather than as something that had arrived.
  It now has a background one step off the buffer's, a border in a colour
  of its own, and `Documentation` written into the top of it. Code inside
  it moved a step further off again, or the panel and the code on it would
  have been the same colour.

  Four faces say all of that — `doc`, `doc-border`, `doc-title` and
  `doc-code` — so a theme can have its own opinion. A face drawn into the
  box that never chose a background is given the panel's; one that did
  keeps it, which is the rule that stops every span punching a hole.

- The README claimed the tree keeps 47 bindings and 41 commands. It keeps
  54 and 48, and has since the root bindings arrived. A test holds the
  number now.

- **The window opens on a 4K display.** The `gui` build asked the GPU for
  the downlevel default limits — the right ask for what it draws, and a cap
  of 2048 pixels on any texture. The surface is a texture. A window filling
  a 3840x2160 display is 3816 across, `Surface::configure` refused it, and
  the editor panicked before it had drawn a frame: a wgpu validation error
  where a window should have been, on the machine where a window was most
  obviously wanted. Half that resolution worked, which is what made it look
  like a size problem rather than a limit. It is the dimension and not the
  area, so 2560x1440 was failing too.

  The resolution now comes from the adapter and nothing else does, so a
  modest GPU is still asked for nothing it cannot give.

## v0.2.5

- **The file tree scrolls with its cursor.** It drew from the window's
  `top_line` and nothing ever moved it, so walking down a project with more
  files than the panel is tall took the cursor off the bottom and left it
  there — invisible, with no way to see where it had got to. Every panel
  went through the same call, so the symbol outline, the buffer list, dired
  and the undo tree were all doing it, and all of them follow their cursor
  now.
- **The tree's root can be moved.** `r d` draws it from the directory under
  the cursor, `r u` from one further out, `r r` from where it opened —
  treemacs' `treemacs-root-down` and `treemacs-root-up`, which it leaves
  unbound. Only the tree moves: the project root a language server is told
  about, and that a project search walks, stays where it was, because
  looking into a subdirectory is not the same as working in a different
  project.

## v0.2.4

- **A configuration to start from.** The install script writes `config.kdl`,
  the four themes and the reference into `~/.config/maxgus` — and never over
  a file that is already there, so upgrading leaves your edits alone. It
  says how many it wrote and how many it left. `--no-config` skips it.
- **An application-menu entry** for the `gui` build: `maxgus.desktop` and an
  icon, installed into `~/.local/share`. The entry carries the binary's
  absolute path, because a launcher does not see the shell's `PATH` and
  `~/.local/bin/maxgus` would otherwise be a menu item that starts nothing.
  Linux and the BSDs only — a `.desktop` file is freedesktop's, and means
  nothing on macOS or Windows. `--no-desktop` skips it.
- A test holds the entry and the script together: the script rewrites the
  `Exec` line, and if the entry ever spells that line differently the
  rewrite would quietly do nothing.

## v0.2.3

- **The install line goes to the address it actually resolves to.**
  `alejandro-llanes.github.io/maxgus-editor/...` was never going to be it:
  the account's user site has a custom domain, so GitHub 301s every project
  path to `http://alejandrollanes.com/...` — and to plain `http`, which is a
  poor hop for something piped into a shell.

  ```sh
  curl -fsSL https://alejandrollanes.com/maxgus-editor/install.sh | sh
  ```

  Nothing else changed. This is a release rather than a note because the
  README inside every v0.2.2 archive carries the old address.

## v0.2.2

- **A feature comparison in the README**, where the build is chosen: a row
  per feature against a column per build, and a test that holds every row —
  a command family is wholly in a build or wholly out of it. `minimal` is
  4.6M and 294 commands with no language server, no autocomplete and no
  grammars in it; `full` is 13M and 443.
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
  curl -fsSL https://alejandrollanes.com/maxgus-editor/install.sh | sh
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
