#!/bin/bash
# Make the app icon: drawn by scripts/make-icon.swift, then every size macOS
# wants. Skips the work while the icon is newer than the drawing.
#   scripts/make-icon.sh   → target/icon/Gibbon.icns
# scripts/bundle.sh and the dev loop (src/bin/dev.rs) use it.
set -euo pipefail
cd "$(dirname "$0")/.."
icon="$(pwd)/target/icon"

if [ ! -f "$icon/Gibbon.icns" ] || [ scripts/make-icon.swift -nt "$icon/Gibbon.icns" ]; then
  mkdir -p "$icon"
  rm -rf "$icon/Gibbon.iconset"
  mkdir -p "$icon/Gibbon.iconset"
  swiftc -O scripts/make-icon.swift -o "$icon/make-icon"
  "$icon/make-icon" "$icon/icon-1024.png" >/dev/null
  for s in 16 32 128 256 512; do
    sips -z "$s" "$s" "$icon/icon-1024.png" \
      --out "$icon/Gibbon.iconset/icon_${s}x${s}.png" >/dev/null
    d=$((s * 2))
    sips -z "$d" "$d" "$icon/icon-1024.png" \
      --out "$icon/Gibbon.iconset/icon_${s}x${s}@2x.png" >/dev/null
  done
  iconutil -c icns "$icon/Gibbon.iconset" -o "$icon/Gibbon.icns"
fi
echo "$icon/Gibbon.icns"
