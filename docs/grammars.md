# Grammars

maxgus is built with eleven tree-sitter grammars in it — **Rust, Python,
JavaScript, JSON, C, HTML, YAML, TOML, INI, XML and Markdown**. Every other
language can be coloured too, by pointing the editor at grammars already
installed on the system. That is what this document is about.

Nothing here is needed to use the editor. A language with no grammar is
edited without colours, which is how it worked before you read this.

- [How it works](#how-it-works)
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

## Turning it on

Nothing is loaded until the configuration says where to look. Add a
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

- **Nothing is loaded unless you said where to look.** There are no default
  directories. An editor with no `grammars` block never opens a library.
- **Only files named for a language you are editing** are opened, in
  directories you named. A directory is not swept.
- **The symbol is derived from the language, not from the file**, so a
  library cannot volunteer what it is called.
- **The result is checked** — non-null, and an ABI version this build can
  read — before it is used to parse anything.
- **Failure is a message**, not a crash: the language goes uncoloured.

This is the same arrangement Neovim and Helix use, and the same one your
package manager already relies on. Point it at `/usr/lib`, or at grammars
you built. Do not point it at a directory anything else can write to.

The code that does it is `crates/maxgus-syntax/src/dynamic.rs`, which is the
one module in the workspace permitted to write `unsafe`, and which says at
length what each of its three `unsafe` operations assumes.
