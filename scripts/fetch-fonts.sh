#!/bin/bash
# Download the bundled fonts (static TTF per weight) and their licenses from
# Google Fonts into assets/fonts/. Run again to refresh them.
#   scripts/fetch-fonts.sh
# Inter and JetBrains Mono come from their own releases; SF Pro and SF Mono
# come from macOS and are not bundled.
set -euo pipefail
cd "$(dirname "$0")/.."
out=assets/fonts
mkdir -p "$out"
# An old user agent makes the Google Fonts API answer with TTF files.
ua="Mozilla/4.0"

# family | license folder in github.com/google/fonts | styles (weight, or weight + i)
fonts=(
  "Geist|geist|400 500 600 700"
  "IBM Plex Sans|ibmplexsans|400 500 600 700"
  "Manrope|manrope|400 500 600 700"
  "DM Sans|dmsans|400 500 600 700"
  "Figtree|figtree|400 500 600 700"
  "Instrument Sans|instrumentsans|400 500 600 700"
  "Geist Mono|geistmono|400 500 700 400i"
  "IBM Plex Mono|ibmplexmono|400 500 700 400i"
  "Fira Code|firacode|400 500 700"
  "Source Code Pro|sourcecodepro|400 500 700 400i"
  "DM Mono|dmmono|400 500 400i"
  "Martian Mono|martianmono|400 500 700"
)

for entry in "${fonts[@]}"; do
  IFS='|' read -r family folder styles <<<"$entry"
  file_base="${family// /}"
  query="${family// /+}"
  for style in $styles; do
    weight="${style%i}"
    if [ "$style" != "$weight" ]; then
      spec="ital,wght@1,$weight"
      name="$file_base-${weight}Italic.ttf"
    else
      spec="wght@$weight"
      name="$file_base-$weight.ttf"
    fi
    url=$(curl -fsS -A "$ua" "https://fonts.googleapis.com/css2?family=$query:$spec" \
      | sed -n 's/.*src: url(\([^)]*\.ttf\)).*/\1/p' | head -1) || true
    if [ -z "$url" ]; then
      echo "skip  $family $style (not available)"
      continue
    fi
    curl -fsS -o "$out/$name" "$url"
    echo "ok    $name"
  done
  curl -fsS -o "$out/$file_base-LICENSE.txt" \
    "https://raw.githubusercontent.com/google/fonts/main/ofl/$folder/OFL.txt"
done
