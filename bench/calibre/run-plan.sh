#!/usr/bin/env bash
S=$(cd "$(dirname "$0")" && pwd)
cd "$S/../.."
tests/e2e/headless.sh python3 "$S/measure.py" "$1" > "$2" 2>&1
echo "exit $?" >> "$2"
