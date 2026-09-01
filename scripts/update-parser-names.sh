#!/usr/bin/env bash
#
# Regenerates the list of parser names compiled into the editor.
#
#     ./scripts/update-parser-names.sh
#
# The editor asks whether to install a grammar when a file's language has
# none. Whether that question is worth asking depends on whether a parser for
# the language exists at all — `zig` has one, `txt` does not — and the answer
# has to be known before any network is touched, because nothing may be
# fetched until the user has agreed to it.
#
# So the *names* ship with the editor and the *repositories* are fetched when
# the user says yes. This writes the names, from the same wiki page the
# editor reads at run time. It is a GitHub wiki, which is a git repository,
# so no scraping is involved.
#
# Run it when the offer stops appearing for a language that has had a parser
# written for it since the last release.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
out="$root/crates/maxgus-syntax/src/parser-names.txt"
wiki=$(mktemp -d)
trap 'rm -rf "$wiki"' EXIT

git clone --depth=1 --quiet https://github.com/tree-sitter/tree-sitter.wiki.git "$wiki"

# The first column of every table row, lower-cased, with the punctuation
# flattened the way the editor flattens it when it matches a language
# against a grammar. The header row and the `| --- |` rule under it go with
# everything that is not a row at all.
awk -F'|' '/^\|/ {
    gsub(/^[ \t]+|[ \t]+$/, "", $2)
    if ($2 == "" || $2 == "name" || $2 ~ /^[-:]+$/) next
    print tolower($2)
}' "$wiki/List-of-parsers.md" \
    | tr '-' '_' \
    | LC_ALL=C sort -u > "$out"   # byte order, which is what Rust compares in

printf '%s parser names written to %s\n' "$(wc -l < "$out")" "${out#"$root"/}"
