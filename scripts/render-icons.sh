#!/usr/bin/env bash
set -eu
cd "$(dirname "$0")/.."
SVG=docs/brand/jadeslip-icon.svg
OUT=app/icons
for spec in 32x32.png:32 128x128.png:128 128x128@2x.png:256 icon.png:512; do
    rsvg-convert -w "${spec#*:}" -h "${spec#*:}" "$SVG" -o "$OUT/${spec%%:*}"
done
