#!/bin/bash
# Check that commit subjects follow Conventional Commits (see CONTRIBUTING.md):
#   <type>(<scope>)!: <description>
# Usage: check-commit-subject.sh "<subject>" ["<subject>" ...]
# Exits 1 and explains when a subject does not match.
set -uo pipefail

types='feat|fix|perf|refactor|style|test|docs|build|ci|chore|revert'
pattern="^($types)(\([a-z0-9][a-z0-9-]*\))?!?: [^ ].*[^.]$"
max=72
status=0

for subject in "$@"; do
  case "$subject" in
    # Messages git writes itself.
    "Merge "* | "Revert \""* | "fixup! "* | "squash! "* | "amend! "*) continue ;;
  esac
  if ! printf '%s' "$subject" | grep -Eq "$pattern"; then
    echo "✗ Not a Conventional Commit: \"$subject\"" >&2
    status=1
  elif [ "${#subject}" -gt "$max" ]; then
    echo "✗ Subject longer than $max characters (${#subject}): \"$subject\"" >&2
    status=1
  fi
done

if [ "$status" -ne 0 ]; then
  cat >&2 <<HELP

  Use:    <type>(<scope>): <description>
  Types:  ${types//|/, }
  Scope:  optional, lower case, e.g. diff, staging, rebase, github
  Break:  add "!" after the type or scope, e.g. feat(settings)!: …
  Example: feat(diff): add a split view
  No period at the end. See CONTRIBUTING.md.
HELP
fi
exit "$status"
