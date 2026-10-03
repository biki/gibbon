#!/bin/bash
# Print the section of one version from CHANGELOG.md, without its heading:
# the notes of its GitHub release (.github/workflows/release.yml). The link
# definitions of the file follow, so that links such as [#12] work there too.
# Usage: changelog-notes.sh <version>     e.g. changelog-notes.sh 0.2.0
# Exits 1 when CHANGELOG.md has no section for the version, or an empty one.
set -euo pipefail

version="${1:?Usage: changelog-notes.sh <version>}"
changelog="$(dirname "$0")/../CHANGELOG.md"

# The lines after "## [<version>]" up to the next "## [" heading, without
# the empty lines at the start and at the end.
notes="$(awk -v heading="## [$version]" '
  /^\[[^]]+\]: / { links = links $0 "\n"; next }
  /^## \[/ { if (found) done = 1; else found = index($0, heading) == 1; next }
  found && !done && (notes != "" || NF) { notes = notes $0 "\n" }
  END { if (notes != "") { sub(/\n+$/, "", notes); printf "%s\n\n%s", notes, links } }
' "$changelog")"

if [ -z "${notes//[[:space:]]/}" ]; then
  echo "✗ CHANGELOG.md has no notes for $version. Add a section \"## [$version] - <date>\"." >&2
  exit 1
fi
printf '%s\n' "$notes"
