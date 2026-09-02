# Grammars

maxgus is built with eleven tree-sitter grammars in it — **Rust, Python,
JavaScript, JSON, C, HTML, YAML, TOML, INI, XML and Markdown**. Every other
language can be coloured too: by letting the editor fetch and build a grammar
for it, or by pointing the editor at grammars already installed on the
system. That is what this document is about.

Nothing here is needed to use the editor. A language with no grammar is
edited without colours, which is how it worked before you read this.

- [How it works](#how-it-works)
- [Letting the editor install one](#letting-the-editor-install-one)
  - [What it runs](#what-it-runs)
  - [When it asks, and when it does not](#when-it-asks-and-when-it-does-not)
  - [Where it puts things](#where-it-puts-things)
  - [When an install fails](#when-an-install-fails)
- [Turning it on](#turning-it-on)
- [Installing grammars](#installing-grammars)
  - [Arch Linux](#arch-linux)
  - [Debian and Ubuntu](#debian-and-ubuntu)
  - [Fedora](#fedora)
  - [macOS, with Homebrew](#macos-with-homebrew)
  - [Nix](#nix)
  - [Windows](#windows)
  - [Building one yourself](#building-one-yourself)
- [Where the queries come from](#where-the-queries-come-from)
- [When it does not work](#when-it-does-not-work)
- [What you are trusting](#what-you-are-trusting)

## How it works

A tree-sitter grammar is a small C library. Compiled, it is a file called
`libtree-sitter-<language>.so` on Linux and the BSDs, `.dylib` on macOS and
`tree-sitter-<language>.dll` on Windows, and it exports one function:
`tree_sitter_<language>`.

Colouring also needs a **highlights query** — a `highlights.scm` that says
which parts of the parse tree are keywords, strings, comments and the rest.
Distributions ship these separately, usually in
`<somewhere>/queries/<language>/highlights.scm`.

Given both, maxgus loads the library when a buffer in that language is
opened, and colours it exactly as it colours a built-in one. Capture names
are mapped to the editor's own faces, so your theme applies without any
extra configuration: `@keyword.function` becomes `font-lock-keyword`,
`@comment` becomes `font-lock-comment`, and so on down the list in
[configuration-reference.md](configuration-reference.md).

**Which grammar wins.** A compiled-in grammar always beats one on disk. A
library cannot quietly replace the eleven the editor was built and tested
with.

**Two that were meant to be built in and are not.** `tree-sitter-kdl` on
crates.io is still bound to tree-sitter 0.20, whose C runtime cannot be
linked beside the one this uses — the symbols collide — and Rhai has no
published grammar crate at all. Both load from disk exactly as described
here, and KDL is worth setting up: it is what this editor's own
configuration is written in.

**Which languages are looked for.** The language of a buffer comes from its
file name. Where nothing is known about an extension, the extension *is* the
language: `main.zig` is `zig`, `init.lua` is `lua`. That is why a grammar
named the usual way is found without listing it anywhere.

## Letting the editor install one

`M-x install-grammar` opens a menu of every parser tree-sitter's own wiki
lists — some five hundred of them — and installing one is choosing a line.
The editor clones the repository, compiles it, puts the library and its
queries where it looks for them, loads it, and colours the buffer. No
configuration is involved.

The other direction is the editor asking. Open a file in a language it has
no grammar for and it says so, naming the repository it would clone:

```
Clone https://github.com/tree-sitter-grammars/tree-sitter-zig, build it and
load it? (yes or no)
```

Answering `no` is the end of it, and it is not asked again for that language
while the editor is running. `set grammar-auto-install=#false` stops it
asking at all; `M-x install-grammar` still works, because the setting governs
the question rather than the feature.

Three commands, and `M-x describe-grammars` to see what came of them:

| Command | What it does |
|---|---|
| `install-grammar` | The menu of every parser there is. |
| `install-grammar-for-buffer` | The one for this buffer's language, which is what a refused offer can be taken up with later. |
| `refresh-grammar-list` | Fetches the list again. It is cached for a week. |

### What it runs

This is the part to read before saying yes, because installing a grammar is
a good deal more than downloading a file:

```sh
git clone --depth=1 <repository> <a temporary directory>
cc -shared -fPIC -O2 -std=c11 -I src src/parser.c src/scanner.c -o libtree-sitter-<language>.so
```

Then the library is `dlopen`ed into the editor's own process. So saying yes
means running that repository's C code on your machine, with your compiler,
in your editor. That is why the question names the repository rather than
the language, and why the whole command line and everything it printed is
kept in a `*Grammar install*` buffer afterwards.

`git` and a C compiler have to be installed. `$CC` and `$CXX` are respected;
without them it uses `cc`, or `c++` for a grammar whose external scanner is
C++. Nothing is run with `sudo` and nothing is written outside your home
directory — a grammar in `~/.local/share` needs no privileges, and using
`sudo` to put it there would leave root-owned files in your own home.

`-Wno-implicit-function-declaration` is passed for C. Scanners written years
ago call `iswspace` without including `<wctype.h>`, which every compiler
accepted until GCC 14; without that flag a good third of the list fails to
build over a missing include its author never had to write.

### When it asks, and when it does not

A file extension the editor knows nothing about *is* the language — `main.zig`
is `zig`, and that is what makes a grammar nobody configured findable at all.
It also means `notes.txt` is `txt`, and a question about installing a `txt`
grammar in front of a file you are typing into would be worse than no
feature.

So the editor only asks when a parser for the language actually exists. The
names of every parser on the wiki ship inside the binary — names only, about
six kilobytes — purely to answer that question before anything is fetched.
`zig` is on it and `txt` is not, so one is offered and the other is silent.
Where the repository *is* comes from the wiki at the moment you ask for it,
so a project that moves is followed without a new release of the editor.

Nothing reaches the network until you have answered a question. On a machine
that has never fetched the list, the first offer asks to look one up; after
that the list is cached and the offer names the repository straight away.

### Where it puts things

`~/.local/share/maxgus/grammars`, in the layout the loader expects:

```
~/.local/share/maxgus/grammars/
├── libtree-sitter-zig.so
├── parser-list.md          ← the cached list, beside rather than among them
└── zig/
    └── highlights.scm
```

That directory is searched without appearing in any configuration file,
because everything in it was put there by an install you agreed to. Nothing
else is searched unless a `grammars` block says so, which is what the rest of
this document is about. A grammar the system already has wins over one
installed here: the configured directories are searched first.

### When an install fails

`M-x describe-grammars` says what happened, and the `*Grammar install*`
buffer has the commands and their output. The usual ones:

| What it says | What happened |
|---|---|
| `` `cc` is not installed `` | There is no C compiler. Install one, or use your package manager's grammar instead. |
| `clone failed` | The repository has moved or gone private. `M-x refresh-grammar-list`, or install it by hand. |
| `has no src/parser.c in it` | The repository ships no pre-generated parser. It needs `tree-sitter generate`, which means node; build it by hand as below. |
| `holds several grammars` | One repository, several languages — `tree-sitter-sfapex` has three. Pick the one you want from `M-x install-grammar`. |
| `built, but it would not load` | It compiled and then `dlopen` refused it. Almost always the ABI: the grammar is older or newer than this build reads. |
| `installed, but neither … nor … has a highlights.scm` | The repository ships no query and Neovim has none for the language either, so it parses and cannot colour. Put a `highlights.scm` in the language's directory yourself. |

A repository that ships no query is not the end of it: the install fetches
Neovim's query for the language from
[nvim-treesitter](https://github.com/nvim-treesitter/nvim-treesitter) and
installs that instead, and the `*Grammar install*` buffer says so. A borrowed
query may have been written against a newer grammar than the wiki's row
points at; patterns naming nodes this grammar does not have are left out
and the rest used, and `M-x describe-grammars` says how many went.

It can also work and look as though it has not. A grammar that extends
another ships a query covering only what it added, so a `.cpp` file with
nothing but its keywords coloured means the query is being read without its
parent — see [Where the queries come from](#where-the-queries-come-from).

## Turning it on

Everything below is the other way in: grammars the system already has, which
need no compiler and no network. Nothing is loaded until the configuration
says where to look. Add a
`grammars` block to `~/.config/maxgus/config.kdl`:

```kdl
grammars {
    // Where libtree-sitter-<language>.so lives.
    search "/usr/lib"

    // Where <language>/highlights.scm lives. Listed in order; the first
    // directory with a query for the language wins.
    queries "/usr/share/tree-sitter/queries"
    queries "/usr/share/nvim/runtime/queries"
}
```

Both are needed: a grammar with no query parses but cannot colour.

For a grammar that is not where those directories would look, name it:

```kdl
grammar "zig" library="/home/me/grammars/libtree-sitter-zig.so" \
              queries="/home/me/grammars/zig/highlights.scm"
```

`queries=` is optional there — without it the `queries` directories are
searched as usual.

**Paths may start with `~`.** No shell reads this file, so the editor expands
a leading `~` or `~/` against `$HOME` itself. Everything else is taken as
written: `~someone` is not another user's home here, and a relative path is
relative to wherever the editor was started, so prefer `~/…` or an absolute
path.

**`M-x describe-grammars`** shows what is built in, what loaded, what did
not and why, and every directory being searched. It is the first thing to
run when something is not coloured.

## Installing grammars

### Arch Linux

The best-served of the lot: grammars are individual packages in the
`tree-sitter-grammars` group, and the queries come with them.

```sh
pacman -S tree-sitter-lua tree-sitter-markdown tree-sitter-vim
# Or the lot:
pacman -S tree-sitter-grammars
```

Libraries land in `/usr/lib`, queries in `/usr/share/tree-sitter/queries`.
The block above works unchanged.

If Neovim is installed, `/usr/share/nvim/runtime/queries` has queries for
several more languages, and they are good ones — keep it in the `queries`
list.

### Debian and Ubuntu

Debian ships a few grammars, and puts libraries in the multiarch directory:

```sh
apt install libtree-sitter-dev
apt-cache search libtree-sitter | grep -v dev     # what is packaged
```

```kdl
grammars {
    search "/usr/lib/x86_64-linux-gnu"
    search "/usr/lib"
    queries "/usr/share/tree-sitter/queries"
}
```

Coverage is thinner than Arch's. For anything not packaged, see
[building one yourself](#building-one-yourself).

### Fedora

```sh
dnf search tree-sitter
dnf install tree-sitter-cli
```

```kdl
grammars {
    search "/usr/lib64"
    queries "/usr/share/tree-sitter/queries"
}
```

### macOS, with Homebrew

Homebrew packages the library and the CLI but not individual grammars, so
the usual route is to build them ([below](#building-one-yourself)) into a
directory of your own:

```sh
brew install tree-sitter
```

```kdl
grammars {
    search "/opt/homebrew/lib"
    search "~/.local/share/maxgus/grammars"
    queries "~/.local/share/maxgus/grammars"
}
```

On an Intel Mac the Homebrew prefix is `/usr/local` rather than
`/opt/homebrew`.

### Nix

```nix
environment.systemPackages = [
  pkgs.tree-sitter
  pkgs.tree-sitter-grammars.tree-sitter-lua
];
```

Nix store paths are not stable across rebuilds, so point at a profile rather
than a store path:

```kdl
grammars {
    search "/run/current-system/sw/lib"
    queries "/run/current-system/sw/share/tree-sitter/queries"
}
```

With Home Manager, `~/.nix-profile/lib` and
`~/.nix-profile/share/tree-sitter/queries`.

### Windows

There is no packaging convention to lean on. Build the grammars you want
([below](#building-one-yourself)) — the CLI produces
`tree-sitter-<language>.dll` — and put them somewhere of your own:

```kdl
grammars {
    search "C:/Users/me/AppData/Local/maxgus/grammars"
    queries "C:/Users/me/AppData/Local/maxgus/grammars"
}
```

Forward slashes are fine in the configuration file and avoid escaping.

### Building one yourself

`M-x install-grammar` does all of this for you and is the easier road. By
hand is worth knowing for a grammar the wiki does not list, one that needs
`tree-sitter generate`, or one you are writing.

Anything not packaged can be built from its grammar repository. It needs the
tree-sitter CLI and a C compiler:

```sh
# The CLI, if it is not already there.
cargo install tree-sitter-cli        # or: pacman -S tree-sitter-cli
                                     # or: brew install tree-sitter

git clone --depth 1 https://github.com/tree-sitter-grammars/tree-sitter-zig
cd tree-sitter-zig
tree-sitter build --output libtree-sitter-zig.so

mkdir -p ~/.local/share/maxgus/grammars/zig
cp libtree-sitter-zig.so ~/.local/share/maxgus/grammars/
cp queries/highlights.scm ~/.local/share/maxgus/grammars/zig/
```

```kdl
grammars {
    search "~/.local/share/maxgus/grammars"
    queries "~/.local/share/maxgus/grammars"
}
```

Grammar repositories live under
[github.com/tree-sitter](https://github.com/tree-sitter) and
[github.com/tree-sitter-grammars](https://github.com/tree-sitter-grammars),
and most carry their own `queries/highlights.scm`.

**Rhai**, for instance, is
[tree-sitter-rhai](https://github.com/rhaiscript/tree-sitter-rhai) through
exactly those steps.

### A word about KDL

This editor's own configuration is KDL, and there is no grammar for it worth
installing yet. The one at
[tree-sitter-grammars/tree-sitter-kdl](https://github.com/tree-sitter-grammars/tree-sitter-kdl)
— including its `update` branch — is **KDL v1**, and maxgus reads **KDL v2**,
where `#true`, `#false` and `#null` are keywords rather than syntax errors.
It builds and loads, and then stops parsing at the first `#true`:

```console
$ printf 'a #true\n' > v2.kdl
$ tree-sitter parse --lib-path libtree-sitter-kdl.so --lang-name kdl v2.kdl
(document (node (identifier) (ERROR)))
```

What that looks like in the editor is the top of the file coloured and the
rest plain, which is worse than none of it coloured, so it is not worth
setting up until a v2 grammar exists. Nothing breaks — a grammar that cannot
parse a file colours what it managed and leaves the rest alone — but nothing
is gained either.

The same caution applies generally: **a grammar has to match the dialect you
write**. If a file colours down to a point and then stops, the grammar and
the file disagree about the language, and the place to look is the grammar's
version rather than this editor.

The name matters. `tree-sitter build` produces a library exporting
`tree_sitter_<name>`, where the name comes from the grammar; maxgus looks
for the language its file name implies. If the two disagree — a
`.zig` file wanting `tree_sitter_zig` from a library that exports something
else — `M-x describe-grammars` says exactly which symbol it could not find.

## Where the queries come from

A grammar and a query are separate things, and the query is what decides how
good the colouring looks. Three usual sources, in rough order of quality:

1. **The grammar's own `queries/highlights.scm`**, in its repository.
2. **Neovim's**, at `/usr/share/nvim/runtime/queries/<language>/`. These are
   maintained, thorough, and use the capture vocabulary maxgus maps from.
3. **The distribution's**, at `/usr/share/tree-sitter/queries/<language>/`,
   which are often the grammar's own copied in.

You can point at all three; the `queries` directories are tried in order and
the first with a `highlights.scm` for the language is used.

**A query can be written on top of another.** `tree-sitter-cpp`'s covers
templates, `namespace` and `co_await` and nothing else — no comments, no
strings, no `int` — because C++ extends C and the query is meant to be read
after C's. A query says so in a comment at the top, which is Neovim's and
Helix's convention and what their query trees are full of:

```scheme
; inherits: c
```

maxgus reads the named language's query in first, from a compiled-in grammar
where there is one and from the same `queries` directories otherwise. Several
parents are comma-separated, brackets around an optional one are ignored, and
a parent that cannot be found costs nothing but the patterns it would have
added.

A repository's own query often lacks the line even when it needs it, because
upstream expects whoever consumes it to know. `M-x install-grammar` writes it
in when the grammar's `grammar.js` says which grammar it extends, and says so
in the install log. If you are installing by hand and a language colours only
its own keywords and nothing a plain C file would have, that missing line is
why.

A query using captures maxgus has no face for is not an error — those parts
simply stay the default colour. The full list of captures that map to
something is in [configuration-reference.md](configuration-reference.md).

## When it does not work

Run `M-x describe-grammars`. It prints the reason, which is one of:

| What it says | What happened |
|---|---|
| `no library named libtree-sitter-x.so in …` | Not in any `search` directory. Check the name and the directory. |
| `… has no tree_sitter_x in it, so it is not an x grammar` | A real library, but not that grammar. Usually the wrong file, or a grammar whose internal name differs from the file name. |
| `… was built for tree-sitter ABI N` | Built against a different tree-sitter than this maxgus reads. Rebuild it with a matching CLI. |
| `no highlights query for x` | The grammar loaded; nothing to colour it with. Add a `queries` directory that has `x/highlights.scm`. |
| `highlights query for x failed to compile` | The query is not well formed. A query merely written for another version of the grammar does not fail like this: its patterns that name nodes the grammar lacks are left out, and the report says how many. |
| `… would not load: …` | The operating system refused it — not a shared library, wrong architecture, or a missing dependency of its own. |

Two things that look like failures and are not:

- **A language with no `grammars` block never loads anything.** The report
  says so in as many words.
- **A grammar is loaded when a file in that language is opened**, not at
  startup. Open one first, then ask.

## What you are trusting

Loading a grammar means loading a shared library into the editor, and a
shared library can run code the moment it is opened. maxgus cannot check
that a file is what it claims to be — nothing can — so it does the next best
things:

- **Nothing is loaded unless you said where to look.** The directories in
  your `grammars` block, and the one the editor installs into — which holds
  only grammars you were asked about by name and agreed to. There are no
  others.
- **Only files named for a language you are editing** are opened, in
  directories you named. A directory is not swept.
- **The symbol is derived from the language, not from the file**, so a
  library cannot volunteer what it is called.
- **The result is checked** — non-null, and an ABI version this build can
  read — before it is used to parse anything.
- **Failure is a message**, not a crash: the language goes uncoloured.

Installing one asks for more than that, and asks for it explicitly.
Compiling a repository's C and loading the result is running its code on
your machine; the question names the repository rather than the language for
exactly that reason, nothing is fetched or built before it is answered, and
what ran is kept in `*Grammar install*` afterwards. `set
grammar-auto-install=#false` if you would rather never be asked.

This is the same arrangement Neovim and Helix use, and the same one your
package manager already relies on. Point it at `/usr/lib`, or at grammars
you built. Do not point it at a directory anything else can write to.

The code that does it is `crates/maxgus-syntax/src/dynamic.rs`, which is the
one module in the workspace permitted to write `unsafe`, and which says at
length what each of its three `unsafe` operations assumes.
