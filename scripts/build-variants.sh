#!/usr/bin/env bash
#
# Builds all three of the editor into one folder, so each can be run and
# compared side by side.
#
#     ./scripts/build-variants.sh            # release, into target/variants
#     ./scripts/build-variants.sh --debug    # faster to build, slower to run
#     ./scripts/build-variants.sh --into ~/bin/maxgus-variants
#
# Each binary says which one it is:
#
#     target/variants/maxgus-minimal --version
#     maxgus 1.4.0 (minimal)
#
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
profile=release
into="$root/target/variants"

while [ $# -gt 0 ]; do
    case "$1" in
        --debug) profile=debug; shift ;;
        --release) profile=release; shift ;;
        --into)
            [ $# -ge 2 ] || { echo "--into needs a directory" >&2; exit 2; }
            into="$2"; shift 2 ;;
        -h|--help) sed -n '2,14p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done

# The three builds, and nothing else: the editor and the treefile alone, the
# whole editor, and the whole editor with a window as well as a terminal.
variants=(minimal full gui)

mkdir -p "$into"
built=()
failed=()
# Where cargo puts what it builds, which is not always `target/`.
target_dir="${CARGO_TARGET_DIR:-$root/target}"
# The logs of a failed build, somewhere nobody else can have put a file.
logs=$(mktemp -d)

for name in "${variants[@]}"; do
    printf '%-8s ' "$name"
    flags=(--no-default-features --features "$name")
    [ "$profile" = release ] && flags+=(--release)
    if cargo build --quiet -p maxgus "${flags[@]}" 2>"$logs/$name.log"; then
        cp "$target_dir/$profile/maxgus" "$into/maxgus-$name"
        size=$(du -h "$into/maxgus-$name" | cut -f1)
        printf 'ok  %6s  %s\n' "$size" "$("$into/maxgus-$name" --version)"
        built+=("$name")
    else
        printf 'FAILED — see %s/%s.log\n' "$logs" "$name"
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
    $into/maxgus-gui FILE       # a window; -nw for the terminal

The same editor with different parts left out; \`--version\` says which. A
key that is not in a build reports itself as undefined rather than doing
nothing.
NOTE
