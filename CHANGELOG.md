# Changelog

## Unreleased

- **What the language server says is set in prose, on a card.** The
  GUI no longer draws the `C-c c k` answer in a box of cells: the
  markdown is set in a proportional face — `set gui-prose-font`, the
  system's sans-serif by default — wrapped at the pixel, with the code
  in the editor's font on a chip, under rounded corners and a border,
  over a blur of the text behind it. It sits under the symbol's line, or
  over it when there is no room, takes at most three fifths of the
  window across and half of it down, and says how many lines it left
  out. The terminal front end draws the box it always has.

- **Pictures open as pictures.** Visiting a PNG, JPEG, GIF, WebP or BMP
  no longer fills a buffer with its bytes: the buffer holds a caption with
  the file's dimensions and size, and the GUI draws the picture under it,
  fitted to the window and kept in its proportions. `C-c o i`
  (`view-image-at-point`) opens the picture a line of markdown or HTML
  refers to, beside the document. A terminal shows the caption and says
  why there is no picture. The minimal build, which has no decoder, reads
  such files as it always did.

- **A mark at the window's edge while it scrolls.** The GUI draws a thin
  bar at the right of each window's text, placed and sized by where the
  window is in its buffer, when the window moves, and fades it out a
  moment after the window stops — so a wheel or a `C-v` says how far there
  is to go and a page being read has nothing at its edge. Off with
  `set gui-scroll-indicator=#false`.

- **The cursor breaks the ligature it lands in.** A block over half of
  `≠` said nothing about which character was under it — whether `DEL`
  would take the `!` or the `=`. The cursor's cell is now drawn as its own
  character, and the characters either side of it are still free to join
  among themselves, so `===` with the cursor in the middle is three `=`
  and `!=` joins again the moment the cursor moves on.

- **A window dragged between a 1× and a 2× display is laid out again.**
  The scale the display reports is kept by the window rather than asked
  for as it goes, the font is cut again at the new size, the padding is
  scaled with it, and the grid is fitted to the window's physical size —
  which is now tested without a display, along with a zoom being a new
  font on the next frame. The documentation said the font size was
  physical pixels; it has always been logical ones, and now says so.

- **The text zooms, in a window.** `C-x C-+` (or `C-x C-=`) draws it a
  tenth larger, `C-x C--` a tenth smaller, `C-x C-0` at its configured
  size again, and the wheel with control held does the same — a notch a
  step for a mouse, a short swipe a step for a touchpad. `M-x
  text-scale-increase`, `text-scale-decrease` and `text-scale-reset` are
  the commands, and a prefix argument is how many steps. The window is
  laid out again for the cells that fit at the new size, the font is cut
  again from the family already loaded rather than looked up on the
  system, and the echo area says where it got to — `Text at 133%`. In a
  terminal the commands say the terminal decides.

- **Room around the text, in a window.** `set gui-line-spacing` opens
  every line up by that many pixels over what the font asks for, half
  above the glyphs and half below so they stay centred in the taller row;
  `set gui-padding` keeps that much margin between the window's edge and
  the text, where the first column used to sit hard against it. Both are
  scaled with the font on a display that reports a scale, and the mouse,
  the input method's candidate window and the blur behind a popup all know
  where the grid moved to.

- **A row that wraps says so, and so does a line that is cut.** With
  `truncate-lines` off, a line that carried on across the rows below read
  the same as two lines that happened to line up; on, a line the edge cut
  off read as a line that ended there. The last column of a window is now
  kept for what a terminal Emacs puts in it — `\` on a row that goes on,
  `$` on a line the edge cuts — in the `fringe` face, and the text stops
  one column short of the edge to leave it room. The position after the
  last character of a line that exactly fills the text columns has that
  column to itself, rather than being held one short.

- **A narrow window's mode line leaves things off rather than cutting
  them.** A tree pane or a thin split used to show `*treefile* 18:0 To`
  with the position cut mid-word. Each piece goes on the bar whole or not
  at all, the buffer's name keeps its end behind an ellipsis when it is
  the piece that will not fit — `…/main.rs` says which file, the front of
  a path does not — and the right-hand group gives way from its inner end,
  problems first and the language last.

- **After staging in magit, point stays where it was.** Staging the last
  unstaged file, or a hunk, took its row away and sent point to the top of
  the buffer, so staging a series of files was a series of trips back
  down. Point lands on the row before the one that went — the previous
  file, the previous hunk, the section's heading — and where a whole
  section went with it, keeps its line.

- **The region can be seen in `maxgus-dark`.** It was two shades off the
  background. Every built-in theme has a `region` colour of its own now,
  and it is a slate blue on the dark theme rather than a guess.

- **dired's marks and directories have faces.** A `D` flag, a `*` mark,
  a directory and a symbolic link used to be the same colour as everything
  else, so a flagged row was found by reading the first column. `dired-flagged`
  (red) and `dired-marked` (yellow) colour the whole row, `dired-directory`
  and `dired-symlink` the name, and `dired-header` the title; all five are
  in the theme and can be set from the configuration.

- **The outline says why it is not shown.** "The outline is not shown" now
  ends with the reason: the `panel-symbols` section is off, the build has no
  language server support, `lsp-enabled` is off, no server is running for
  the language, or the buffer has no language to ask a server about.

- **`C-x b` highlights its default.** The popup's highlight sat on the
  buffer being left, while the prompt named another buffer as the default
  the empty answer would take. The buffer being left is last in the list
  now, so the highlight and the default agree, and `RET` does what the
  screen says.

- **Small words.** "1 lines" and "1 bytes" are singular, and "2 file(s)",
  "3 match(es)" and "1 director(ies)" are counted out — "2 files", "3
  matches", "1 directory" — in every message that used to hedge. A snippet field
  alone on an indented blank line — the body of `fn` — used to leave the
  indentation behind as trailing whitespace; the line is left empty until
  the field is reached, and indented then.

- **In the window: the clipboard is wired to the kill ring.** `C-w` and
  `M-w` used to keep the text to themselves, and `C-y` never looked outside;
  only a mouse selection reached the system clipboard, and it overwrote it
  on every release of the button. A kill now goes to the clipboard as
  well, and a yank takes what another program has put there since — into
  the kill ring, so `M-y` walks back past it, and only once, so yanking the
  same text twice does not fill the ring with copies. A mouse selection
  goes to the primary selection instead, where the platform has one, and
  the middle button pastes from there.

- **In the window: input methods and dead keys.** Composed text — `´` then
  `e`, a compose sequence, an input method's Japanese — used to be dropped,
  since only single characters were read as keys. Whatever arrives as text
  is inserted as text, and the input method's candidate box is placed at
  the cursor.

- **In the window: a character the font lacks is drawn from one that has
  it.** CJK, emoji and symbols outside the configured family used to draw
  as boxes. The system's fonts are searched for one that has the glyph —
  the Nerd and Noto families first, then any monospace, then anything — and
  it is scaled to fit one cell, or two for a wide character. A colour emoji
  font keeps pictures rather than outlines, and those are drawn as the
  pictures they are, in their own colours.

- **In the window: the glyph atlas grows.** It was one texture of a fixed
  size, and when a big font or a high-DPI display filled it, every glyph
  after that was silently dropped and text went missing. It doubles as
  needed, up to what the GPU allows.

- **In the window: bold and italic come from the configured family.** The
  fallback for a style the family lacked was the next family's bold — so
  `gui-font "Noto Sans Mono"` drew comments in DejaVu's italic — and
  `monospace` was never matched at all. Styles are taken from the family
  that has a regular face and from nowhere else, `monospace` means the
  system's monospace, and the cell is wide enough for the widest style.

- **In the window: remembered size, a hollow cursor when unfocused, files
  dropped on it, more of the mouse.** The window opens at the size it was
  last closed at, kept in `window.kdl` in the state directory beside the
  sessions. When another window has the keyboard the cursor is an outline
  rather than a block, as Emacs' is. A file dropped on the window is
  opened. A double click selects the word and a triple the line, and the
  right button stretches the region to where it landed.

- **The grammar offer can be declined from the menu.** The question "install
  from?" was a completion prompt, so `n` narrowed the list instead of saying
  no and there was nothing to pick but a source; `C-g` was the only way
  out. A `skip` row is the last candidate now, and the official
  `tree-sitter-grammars` source no longer ranks below a fork with a longer
  name when you type part of one.

- **A grammar with no highlights query borrows Neovim's.** A repository
  that ships a parser and no `queries/highlights.scm` used to install as a
  grammar that could not colour, with a warning telling you to write one.
  The install now fetches the query for the language from nvim-treesitter,
  and says so in `*Grammar install*`; the warning stays for a language
  Neovim has no query for either. Two things make a borrowed query usable:
  a pattern that names a node this version of the grammar does not have is
  left out rather than failing the whole query — `M-x describe-grammars`
  says how many went — and a pattern resting on a predicate this editor
  cannot evaluate, such as Neovim's `#lua-match?`, is switched off rather
  than matching everything. A query that will not compile at all is
  reported there too, where it used to fail silently.

- **Special buffers name their mode.** Dired, magit, the terminal, `*Help*`,
  `*Occur*` and `*xref*` all called themselves `Fundamental` in the mode
  line, the buffer list and `C-h m`. They say `Dired`, `Magit`, `Terminal`,
  `Help` and so on; `Fundamental` is kept for a buffer with no mode at all.

- **`occur`, the language server's lists and `*Help*` open beside the text
  rather than over it, and lead somewhere.** All three used to replace the
  window you were working in with a read-only buffer that answered to
  nothing: no way to visit a match, no `q` to put it away. They are modes
  now. A listing opens in the other window — or a new one below, when there
  is only one — with `n`/`p` to walk its rows, `RET` to visit the row's place
  in the window you came from, `o` to visit it and stay in the list, and `q`
  to close it. The matches in an `occur` listing are highlighted, `*xref*`
  puts the language server's definitions, references and symbols behind the
  same keys, and `*Help*` has `q` and nothing else, which is all it needed.

- **`M-g g` and `M-g c` ask for the line or character.** Without a prefix
  argument they used to fail and leave the digits you then typed in the
  buffer. `C-u 42 M-g g` still goes straight there.

- **`C-h k` reads a whole key sequence.** `C-h k C-x C-f` used to answer
  "`C-x` is a prefix key" and then run `C-f`; now it keeps reading, echoing
  what it has so far, until the sequence names a command or nothing.

- **The cursor is drawn where point is in magit buffers**, which have no
  line-number gutter to add the width of. It used to sit several cells to
  the right of the character it was on.

- **The cursor stays on screen at the end of a long truncated line** with
  line numbers on. Horizontal scrolling measured the window's full width
  while the text was drawn after the gutter, so `End` on a long line left
  point past the edge.

- **A message clears on the next keystroke**, as Emacs' do. "Mark set" and
  the red text from a failed command used to stay in the echo area for as
  long as nothing else replaced them.

- **`query-replace` highlights the occurrence it is asking about** and says
  what it is replacing with what: `Query replacing foo with bar: (y, n, !,
  q, .)`.

- **A transient menu keeps each group with its heading.** The two-column
  layout used to cut the list by row count, so "Log" could end up at the
  bottom of the left column with its items starting the right one. Groups
  are packed whole into as many columns as fit, and a group is never split.

- **The completion popup shows as much of a candidate as it has room for.**
  Names were cut at a fixed width whatever the popup's — grammar sources
  read `github.com/tree-sitter-gramma` — and the list stopped at fifteen
  rows with the screen half empty. A candidate with no annotation may use
  the whole width; one with an annotation gets two thirds of it; and the
  popup grows to half the frame.

- **Typing with several cursors no longer scrolls the window to the last
  of them.** Each cursor's edit followed its cursor, so the window ended
  wherever the furthest one was; the view is put back where it started and
  only moves if the real point has left it.

- **A read-only buffer says "Buffer `*dired*` is read-only"** rather than
  `text error: io error: buffer is read-only`.

- **A documentation popup goes away with the buffer it was about.** A hover
  reply that arrived after `M-.` had moved point elsewhere was shown over
  whatever was on screen; a reply for somewhere point has left is dropped.

- **Snippets are found in `snippets/rust/` as well as `snippets/rust-mode/`.**
  The README said the former and only the latter worked, so `fn TAB`
  silently indented instead of expanding.

- **In the window: ligatures only in code.** The font was joining `->` in a
  help page, `--color` on a shell line and `M--` in the list of bindings,
  none of which are the arrow, the em dash or the ligature the font had in
  mind. Ligatures now form in windows showing code — a file whose language
  is not prose — and nowhere else.

- **In the window: box-drawing and block characters are drawn as shapes.**
  `█` came from the font with a seam at every cell and a strip of
  background along the top, and a terminal program's `┌─┐` frames had gaps
  at the corners. The two blocks (U+2500–U+259F, bar the three diagonals)
  are drawn as rectangles that fill exactly the cell they are given: light,
  heavy and double lines with their corners and crossings, dashes, halves,
  eighths, quadrants and the three shades.

- **In the window: a line between windows side by side.** `C-x 3` used to
  put two windows' text edge to edge with nothing between them. A thin
  divider runs down the seam, in the new `vertical-border` face.

- **In the window: `dim` and `strikethrough` faces are drawn.** Both were
  ignored; `tree-git-ignored` is dim and was not.

- **In the window: a wide character's background covers both its cells.**
  The region, the mode line and a search match used to colour the first
  cell of a CJK character or an emoji and leave the second plain.

## v1.3.1

- **`; inherits: c` in a highlights query is read.** A grammar that extends
  another ships a query covering only what it added: `tree-sitter-cpp`'s has
  templates, `namespace` and `co_await` in it and no comments, no strings and
  no `int`, because it is meant to be read after C's. Used alone it coloured a
  handful of C++ keywords and left the rest of the file plain, which looks
  like a grammar that does not work rather than half a query — and it is what
  a `.cpp` file did after `M-x install-grammar`.

  The directive is Neovim's and Helix's, and their query trees — the ones
  `queries "/usr/share/nvim/runtime/queries"` points at — are full of it, so
  this was wrong for the configured road as well as the installed one. The
  inherited query is read in front of the one inheriting it, from a
  compiled-in grammar where there is one and from the `queries` directories
  otherwise. Cycles stop, and a parent nobody has costs only the patterns it
  would have added.

- **An installed grammar is told what its query is written on top of.**
  Upstream repositories mostly do not carry the directive, because they
  expect whoever consumes the query to know. `M-x install-grammar` now reads
  the grammar's own `grammar.js` — C++ opens with
  `require('tree-sitter-c/grammar')` — and writes `; inherits: c` into the
  installed query when it says so, noting it in the install log. A query that
  already says what it inherits is left alone.

## v1.3.0

- **The editor can fetch and build a grammar for you.** `M-x install-grammar`
  lists every parser on tree-sitter's own wiki — five hundred of them — and
  installing one is choosing a line: it is cloned, compiled, put where the
  loader looks and loaded, with no configuration written anywhere. Opening a
  file in a language with no grammar offers the same thing and names the
  repository it would clone, because saying yes means running that
  repository's C on your machine. `set grammar-auto-install=#false` stops the
  asking; `M-x install-grammar` still works, since the setting governs the
  question rather than the feature. `M-x install-grammar-for-buffer` takes up
  a refused offer later and `M-x refresh-grammar-list` fetches the list
  again.

  Nothing reaches the network before a question has been answered, and the
  question is only asked when there is something to offer. The *names* of the
  parsers ship in the binary — six kilobytes, no repositories — purely so
  that `main.zig` can be offered a grammar while `notes.txt` is left alone;
  a file extension nothing is known about *is* the language here, so without
  that list a `.txt` file would be asked about as readily as a `.zig` one.
  Where a parser lives is read from the wiki when you ask for it, so a
  repository that moves is followed without a new release.

  Installs go to `~/.local/share/maxgus/grammars`, which is searched without
  appearing in any configuration file — everything in it was put there by an
  install you agreed to. The configured directories are still searched first,
  so a grammar your package manager installed is not shadowed by one built
  here. `M-x describe-grammars` says what loaded, and a `*Grammar install*`
  buffer keeps every command that was run and everything it printed.

  `git` and a C compiler are what it needs, and it uses `cc` — `$CC` and
  `$CXX` are respected, `c++` is used for a C++ scanner, macOS gets
  `-dynamiclib`, and nothing is ever run with `sudo`. C is compiled with
  `-Wno-implicit-function-declaration`, because scanners written before GCC
  14 call `iswspace` without including `<wctype.h>` and a third of the list
  would otherwise fail to build over an include its author never had to
  write.

- **`~` in a grammar path from the configuration is your home directory.**
  `search "~/.local/share/maxgus/grammars"` was taken literally: no shell has
  read that file, so nothing had expanded it, and the editor looked for a
  directory called `~` under wherever it was started — which exists nowhere,
  so the grammar it named was reported as simply missing. The documentation
  had been recommending exactly that spelling. `~` and `~/…` now expand
  against `$HOME` for every path in a `grammars` block. `~someone` is left
  alone: another person's home directory needs the password database rather
  than an environment variable.

- **A grammar is found whichever way its name is punctuated.** A `.cs` file
  is `c-sharp` here and the grammar that colours it calls itself `c_sharp`,
  and the two never met. Both spellings are now tried, for the library and
  for the query directory beside it.

## v1.2.0

- **`truncate-lines` off now wraps.** It only ever turned off the horizontal
  scroll: long lines were still clipped at the edge, so the setting and
  `C-c t w` both claimed something the editor did not do. A line too long for
  its window now carries on across the rows below it, breaking where the edge
  falls, with its number drawn once against the first row. Everything that had
  assumed a row was a line was taught otherwise — the cursor, the extra
  cursors of a multiple-cursor edit, the beacon, the region highlight, and the
  scrolling, where a page is now a screenful of *rows* rather than of lines
  that might each take three.
- **`C-s` in that box searches every directory under your home directory.**
  Walking is the wrong way to reach somewhere that is not under where you
  started. The box takes the whole tree at once, listed by path relative to
  home, and typing narrows across all of it — `maxgused` finds
  `Projects/personalProjects/maxgus-editor` out of five thousand. Dotfiles,
  `node_modules`, `target` and their like are walked past. `←` comes back out.
- **`RET` on `..` in the box that asks the tree for a directory goes up**
  rather than answering with the parent. It is the row you press to get out of
  somewhere, and answering with it left you having added a directory you were
  only passing through. To choose a parent, go up and press `.`.

## v1.1.0

- **The tree asks which directory by showing you one.** `r a` and `C-x t d`
  wanted a path typed out in full. They open the file browser instead, on the
  directory the tree is already in: only directories are listed, so every row
  is an answer, `→` goes in, `←` comes out, and `RET` chooses. `.` leads the
  list and answers with the directory being looked at, so adding the one you
  started in is one keypress. A path can still be typed or pasted — a filter
  with a `/` in it is taken literally rather than searched for — and `~` now
  means home at both, which it did not before.

## v1.0.0

The editor has been usable for a while; this is the release that says so. The
number is a promise about what happens next rather than a claim that anything
suddenly got better — the keys, the command names, the configuration file and
the faces are what they are now, and breaking any of them is a major version
from here.

What arrived with it:

- **The pseudo-terminal the smoke tests drive was blocking, and one loop
  never drained it.** `wait_until_stopped` polled the editor's state for
  twenty seconds while reading nothing, so an editor that filled the
  terminal's buffer on the way to suspending blocked in `write` and never
  reached the `C-z` it had been sent. It failed as `never stopped; it is in
  state S`, only under load, and only in the one test that suspends.

  The descriptor is non-blocking now — which is what the code already
  claimed, since a read that reports "nothing to read yet" is not something
  a blocking descriptor does — and every loop that waits also drains. The
  smoke suite is a third quicker for it, because `settle` now keeps to the
  250ms it was written to take instead of blocking past it.

- **A review across every build, and what it found.** The three builds are
  a promise the README makes in a table, and most of that table had nothing
  holding it up.

  Two features that ship in *every* build were missing from it entirely —
  several directories in the tree with workspaces, and the file browser —
  so the table people choose a build from did not mention them. The binary
  sizes were stale: measured, they are 4.7M, 12.2M and 20.7M against the
  4.6M, 13M and 20M written down. The `minimal` column's command count had
  no test at all, where the `full` ones have had since they were written; it
  turned out to be right, and is now checked.

- **Bindings and commands are held together per build.** Both lists are
  feature-gated, in two different files, and nothing was checking that they
  agree — a binding left on the wrong side of a `cfg` is a documented key
  that reports `unknown command`, in the build nobody runs. Every map is
  checked against the registry now, including the fallback a map may have
  for keys it did not name. Both builds were already correct.

- **The file browser and workspaces are exercised against a real terminal**,
  in whatever build the suite is run in: the browser narrows, walks in and
  out of directories and opens a file; a workspace is saved, the editor is
  closed, and a fresh one opens it by name. The workspace test gets its own
  state directory, so it neither reads nor writes the real one.

- **Workspaces: a set of directories, named and kept.** The tree can show
  several at once; a workspace is that list given a name and written down,
  so the set you work in is one command to come back to rather than several
  to rebuild. treemacs has the same idea and the same word for it.

  **`M-x workspace-delete` is a list to pick from**, not a name to type.
  The popup is the question: it opens with the first row under the cursor,
  the arrows walk it, and `RET` forgets the row it is pointing at. It asked
  for the name in full before, on the grounds that a prompt which deletes
  whatever it is pointing at deletes things by accident — but what it
  deletes is a list of directories and not the directories, the row is on
  screen while it is being chosen, and saving it again takes a name and a
  `RET`. The caution was not worth what it cost to answer.

  Saving is something you do rather than something that happens: `r a` and
  `r k` change the tree in front of you and nothing else, so the tree is
  somewhere to rearrange freely and the workspace on disk is what it was
  when you last named it. `C-c p s` offers the name it was opened under, so
  keeping a change is `RET`.

  `C-c p s` saves what the tree is showing, `C-c p p` opens a saved one and
  `C-c p d` forgets one. The switch prompt offers a default — the one
  already open, or the first saved — so `RET` on it means something; the
  delete prompt deliberately offers none, because a prompt that deletes
  whatever it was pointing at when `RET` was hit by accident is a prompt
  that deletes things by accident. Opening a workspace **opens the tree if it is
  closed** — a set of directories to look at is nothing without somewhere to
  look at them. Deleting one forgets a list, not the directories on it.

  Kept as KDL beside the sessions, and read at startup whether or not a
  session is being restored: a session is where you left off and a workspace
  is what you are working on, so one is restored for you and the other is
  chosen. A directory that has moved since it was saved is left out and said
  out loud, because a workspace outlives the disk it was saved on and
  silently showing three of four is how someone comes to think they deleted
  something.

- Two tests were sharing a temporary directory, and its `Drop` removes it —
  so whichever finished first deleted the ground out from under the other.
  It surfaced as an intermittent failure in a file that had nothing to do
  with either of them. They have their own now, and the fixture says why.

- **Eight glyphs were drawing as hollow boxes and nothing said so.** Nerd
  Fonts v3 moved the Material Design icons out of `U+F534..U+FD46` and up to
  `U+F0001`, leaving the old codepoints unassigned — and eight of the
  editor's were still down there. `DIRECTORY_OPEN` was one, so *every open
  directory in the tree* had a box where its glyph should be, on any machine
  with a font from the last few years. A test refuses that range now; it
  would have caught all eight.

- **The symbol outline uses one set of icons.** What it had was a handful
  from Font Awesome, a handful from Material and a handful from an extension
  pack, which reads as a ransom note even where the font has all of it —
  and seven of them it did not, including `method`, `string`, `number` and
  `boolean`. They are Codicons now: the icons the editor that invented the
  protocol drew for `SymbolKind`, so an outline here looks like an outline
  anywhere else.

- **The tree marks what can be opened with a chevron** rather than with `>`
  and `v`, which are letters pretending to be arrows. The symbol outline
  does the same. `set nerd-font-icons=#false` still gets the letters, and
  the mark is two columns wide either way so nothing shifts.

- **A file browser you type at**, on `C-x C-d`. `C-x C-f` is for when you
  know the path; this is for when you know roughly where it is. A box over
  the frame that narrows fuzzily as you type, walked with the arrows —
  right goes into a directory, left comes back out, backspace rubs out a
  character or goes up when there is none. Filetype glyphs, sizes and dates.
  It is not dired and does not touch it: dired is for working *on* a
  directory, and its single-letter keys are what make that quick.

- **The tree and the symbol outline read as trees.** A rule down each level
  of nesting, so the shape is drawn rather than measured out of whitespace,
  and a bar down the left of the selected row — the background says *that*
  something is selected, the bar says *where* when the eye is elsewhere.
  `tree-indent` and `tree-selection-mark`.

- **The tree shows more than one directory.** `r a` adds another and `r k`
  takes one off — treemacs' projects, and for its reason: a workspace is
  usually more than one directory, and closing the tree to reopen it
  somewhere else is not having both.

  The first one stays *the project*. What a language server is told about
  and what a project search walks does not move because you asked to look
  at something else as well, which is the same line `r d` already drew
  between the tree's root and the project's.

  `r d` and `r u` move whichever directory the cursor is in and leave the
  others alone. They used to rebuild the whole tree around the new root,
  which would have thrown away every other directory on the list — a
  surprising amount to lose from a command that says it moves *the* root.
  The last directory cannot be removed: a tree with nothing in it has no row
  to put a cursor on and no way to ask for a directory back.

- **`visit-theme` is `consult-theme` now, and it stops asking.** The name
  first: every command here is named after the one it behaves like, so that
  a hand which already knows the keys knows the names too — and this one
  behaves like `consult-theme`, so that is what it is called.

  What it stopped doing is asking. It applied each theme as it came under
  the cursor, and then, once you had chosen one by *looking at it*, put a
  yes-or-no question about the configuration file between you and the theme
  you had just picked. Trying themes on and deciding to keep one for good
  are two different intentions and only the first is what the command is
  for. `RET` now keeps what is showing and that is the end of it.

  Keeping one is its own command: **`save-theme`** writes the theme in use
  into the configuration file — whatever theme is in use, however it got
  there, so one arrived at by `load-theme` is kept the same way. A prefix
  argument on `consult-theme` does both at once. Writing still changes that
  one setting and leaves the rest of the file alone.

  A name that is not a theme now leaves the theme you started with, rather
  than accepting the preview and then reporting the error — the preview has
  already changed the screen by the time `RET` is pressed, so there was
  something to undo and it was not being undone.

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
