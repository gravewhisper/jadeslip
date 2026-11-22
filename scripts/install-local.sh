#!/usr/bin/env bash
set -eu

if [ "$#" -lt 1 ]; then
    echo "usage: scripts/install-local.sh <folder> [extra cargo args for the app]" >&2
    exit 2
fi

DEST="$1"
shift
cd "$(dirname "$0")/.."
cargo install --locked --force --path app --root "$DEST" "$@"
cargo install --locked --force --path crates/worker --root "$DEST"
install -D -m 644 crates/core/dict/README_en_US.txt "$DEST/share/doc/jadeslip/README_en_US.txt"
install -D -m 644 LICENSE "$DEST/share/doc/jadeslip/LICENSE"
echo "installed: $DEST/bin/jadeslip and $DEST/bin/jadeslip-worker"
