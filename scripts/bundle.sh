#!/bin/bash
# Build Gibbon.app: release binary, Info.plist, icon, signature.
#   scripts/bundle.sh      → target/release/bundle/Gibbon.app
# Signs with $GIBBON_SIGN_IDENTITY, else a "Developer ID Application"
# identity, else the "Gibbon Release" certificate of
# scripts/make-signing-cert.sh, else ad hoc, which runs on this Mac.
set -euo pipefail
cd "$(dirname "$0")/.."
root="$(pwd)"

cargo build --release

app="$root/target/release/bundle/Gibbon.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/gibbon "$app/Contents/MacOS/Gibbon"

# Icon: drawn by scripts/make-icon.swift, then every size macOS wants.
icon="$root/target/icon"
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
cp "$icon/Gibbon.icns" "$app/Contents/Resources/Gibbon.icns"

version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Gibbon</string>
  <key>CFBundleDisplayName</key><string>Gibbon</string>
  <key>CFBundleIdentifier</key><string>dev.gibbon.Gibbon</string>
  <key>CFBundleExecutable</key><string>Gibbon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>${version}</string>
  <key>CFBundleIconFile</key><string>Gibbon</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST

identity="${GIBBON_SIGN_IDENTITY:-$(security find-identity -v -p codesigning 2>/dev/null \
  | sed -n 's/.*"\(Developer ID Application:[^"]*\)".*/\1/p' | head -1)}"
# Self-signed, so not in the list of valid identities (-v).
if [ -z "$identity" ] \
  && security find-identity -p codesigning 2>/dev/null | grep -q '"Gibbon Release"'; then
  identity="Gibbon Release"
fi
if [ -n "$identity" ]; then
  codesign --force --options runtime --timestamp --sign "$identity" "$app"
else
  codesign --force --sign - "$app"
fi
echo "$app"
