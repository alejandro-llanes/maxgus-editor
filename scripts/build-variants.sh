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
#     maxgus 0.1.0 (minimal)
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

# The three builds, and nothing else: the editor and the treefile alone, the
# whole editor, and the whole editor with a window as well as a terminal.
variants=(minimal full gui)

mkdir -p "$into"
built=()
failed=()

for name in "${variants[@]}"; do
    printf '%-8s ' "$name"
    flags=(--no-default-features --features "$name")
    [ "$profile" = release ] && flags+=(--release)
    if cargo build --quiet -p maxgus "${flags[@]}" 2>"/tmp/maxgus-variant-$name.log"; then
        cp "$root/target/$profile/maxgus" "$into/maxgus-$name"
        size=$(du -h "$into/maxgus-$name" | cut -f1)
        printf 'ok  %6s  %s\n' "$size" "$("$into/maxgus-$name" --version)"
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

The same editor with different parts left out; \`--version\` says which. A
key that is not in a build reports itself as undefined rather than doing
nothing.
NOTE
