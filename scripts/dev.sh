#!/bin/bash
# Dev loop: rebuild and restart Gibbon on every save (see src/bin/dev.rs).
#   scripts/dev.sh [repo]
# In the running debug app, ⌘⌥I opens the GPUI inspector.
set -euo pipefail
cd "$(dirname "$0")/.."
exec cargo run --quiet --bin dev -- "$@"
