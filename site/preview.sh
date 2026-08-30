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
port="${1:-8000}"
echo "http://127.0.0.1:$port"
cd "$here" && python3 -m http.server "$port"
