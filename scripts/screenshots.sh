#!/bin/bash
# Take the README screenshots again: build a demo repository with agent
# branches, open Gibbon on it in each view, in dark and in light, and write
# docs/screenshots/<view>-<dark|light>.png.
#   scripts/screenshots.sh [view...]   # history changes pick review activity cleanup settings
# Gibbon windows pop up on top for about two minutes and take no focus. Keep
# the display awake. The first run installs Pillow and imagequant into
# target/screenshots/venv. The demo is in scripts/screenshots/demo.py.
set -euo pipefail
cd "$(dirname "$0")/.."
work="$(pwd)/target/screenshots"
mkdir -p "$work"

cargo build --release
for tool in winpid frame; do
  if [ ! -x "$work/$tool" ] || [ "scripts/screenshots/$tool.swift" -nt "$work/$tool" ]; then
    swiftc -O "scripts/screenshots/$tool.swift" -o "$work/$tool"
  fi
done
if [ ! -x "$work/venv/bin/python" ]; then
  python3 -m venv "$work/venv"
  "$work/venv/bin/pip" install --quiet pillow imagequant
fi
exec "$work/venv/bin/python" scripts/screenshots/take.py "$work" "$@"
