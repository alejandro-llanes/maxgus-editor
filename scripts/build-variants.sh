#!/usr/bin/env bash
#
# Builds every feature combination into one folder, so each can be run and
# compared side by side.
#
#     ./scripts/build-variants.sh            # release, into target/variants
#     ./scripts/build-variants.sh --debug    # faster to build, slower to run
#     ./scripts/build-variants.sh --into ~/bin/maxgus-variants
#
# Each binary says what is in it:
#
#     target/variants/maxgus-git --version
#     maxgus 0.1.0 (git)
#
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
profile=release
into="$root/target/variants"

while [ $# -gt 0 ]; do
    case "$1" in
        --debug) profile=debug; shift ;;
        --release) profile=release; shift ;;
        --into) into="$2"; shift 2 ;;
        -h|--help) sed -n '2,14p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done

# Name, then the features it is built with. `minimal` is the editor and the
# treefile alone; the middle rows are each subsystem on its own, which is what
# the feature flags are for; `full` and `gui` are what a release ships.
variants=(
    "minimal:minimal"
    "syntax:syntax"
    "lsp:lsp"
    "git:git"
    "terminal:terminal"
    "grep:grep"
    "script:script"
    "lsp-git:lsp,git"
    "full:full"
    "gui:gui"
)

mkdir -p "$into"
built=()
failed=()

for entry in "${variants[@]}"; do
    name="${entry%%:*}"
    features="${entry#*:}"
    printf '%-10s %-24s ' "$name" "[$features]"
    flags=(--no-default-features --features "$features")
    [ "$profile" = release ] && flags+=(--release)
    if cargo build --quiet -p maxgus "${flags[@]}" 2>/tmp/maxgus-variant-$name.log; then
        cp "$root/target/$profile/maxgus" "$into/maxgus-$name"
        size=$(du -h "$into/maxgus-$name" | cut -f1)
        printf 'ok   %6s  %s\n' "$size" "$("$into/maxgus-$name" --version)"
        built+=("$name")
    else
        printf 'FAILED — see /tmp/maxgus-variant-%s.log\n' "$name"
        failed+=("$name")
    fi
done

echo
echo "${#built[@]} built into $into"
if [ "${#failed[@]}" -gt 0 ]; then
    echo "${#failed[@]} failed: ${failed[*]}" >&2
    exit 1
fi
cat <<NOTE

Run one:

    $into/maxgus-minimal FILE
    $into/maxgus-full FILE
    $into/maxgus-gui --gui FILE

Each is the same editor with different parts left out; \`--version\` says
which. A key that is not in a build reports itself as undefined rather
than doing nothing.
NOTE
