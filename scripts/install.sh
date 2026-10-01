#!/bin/bash
# Install Gibbon from the latest GitHub release, or update it:
#   curl -fsSL https://raw.githubusercontent.com/biki/gibbon/main/scripts/install.sh | bash
# Puts Gibbon.app in /Applications, or in $GIBBON_DIR:
#   curl -fsSL …/install.sh | GIBBON_DIR=~/Applications bash
# A download with curl has no quarantine mark, so macOS opens Gibbon without
# the Gatekeeper prompt, which every app without notarization gets from a
# browser download.
set -euo pipefail
repo=biki/gibbon
dir="${GIBBON_DIR:-/Applications}"
app="$dir/Gibbon.app"

if [ "$(uname -s)" != Darwin ] || [ "$(uname -m)" != arm64 ]; then
  echo "The releases are for Macs with Apple Silicon. Build Gibbon from source:" >&2
  echo "  https://github.com/$repo#getting-started" >&2
  exit 1
fi
mkdir -p "$dir"
if [ ! -w "$dir" ]; then
  echo "You cannot write to $dir. To install Gibbon in your own Applications folder:" >&2
  echo "  curl -fsSL https://raw.githubusercontent.com/$repo/main/scripts/install.sh | GIBBON_DIR=~/Applications bash" >&2
  exit 1
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp" "$dir/.Gibbon.app.new"' EXIT
echo "Downloading the latest release of Gibbon…"
curl -fL --progress-bar -o "$tmp/Gibbon.zip" \
  "https://github.com/$repo/releases/latest/download/Gibbon.zip"
ditto -x -k "$tmp/Gibbon.zip" "$tmp"

# Each file is as the release certificate signed it, and the signature is
# not ad hoc. This script does not compare the certificate with the one of
# an installed Gibbon: a new certificate needs this install.
if ! codesign --verify --deep --strict "$tmp/Gibbon.app" 2>/dev/null \
  || ! codesign --display -r- "$tmp/Gibbon.app" 2>/dev/null | grep -q 'certificate leaf'; then
  echo "The download has no valid signature. Gibbon is not installed." >&2
  exit 1
fi
version="$(plutil -extract CFBundleShortVersionString raw -o - "$tmp/Gibbon.app/Contents/Info.plist")"

# Copy next to the old app first, so a failed copy keeps the old app.
ditto "$tmp/Gibbon.app" "$dir/.Gibbon.app.new"
rm -rf "$app"
mv "$dir/.Gibbon.app.new" "$app"

echo "Installed Gibbon $version in $dir."
if pgrep -x Gibbon >/dev/null; then
  echo "Quit Gibbon and open it again to use the new version."
else
  echo "Open it with: open \"$app\""
fi
