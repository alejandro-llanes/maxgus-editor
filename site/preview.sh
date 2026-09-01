#!/usr/bin/env bash
#
# Assembles the site the way the Pages workflow does and serves it, so it
# can be looked at before it is published.
#
#     ./site/preview.sh          # http://127.0.0.1:8000
#     ./site/preview.sh 9000
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
mkdir -p "$here/screenshots"
cp "$here/../docs/screenshots/"* "$here/screenshots/"
# The demo clips, when there are any. `scripts/record-demos.sh` makes them.
if [ -n "$(ls -A "$here/../docs/media" 2>/dev/null)" ]; then
    mkdir -p "$here/media"
    cp "$here/../docs/media/"* "$here/media/"
fi
port="${1:-8000}"
echo "http://127.0.0.1:$port"
cd "$here" && python3 -m http.server "$port"
