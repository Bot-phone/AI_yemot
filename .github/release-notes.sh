#!/usr/bin/env bash
# Compose the GitHub release body for one version:
#   1. the Hebrew usage guide in desktop/RELEASE_GUIDE.md (every release, first)
#   2. "מה חדש" — the matching "## <version>" section of CHANGELOG.md
# Usage: .github/release-notes.sh <version> > body.md
set -euo pipefail
version="${1:?version}"
root="$(cd "$(dirname "$0")/.." && pwd)"

cat "$root/desktop/RELEASE_GUIDE.md"
echo
echo "---"
echo
echo "## מה חדש בגרסה ${version}"
echo
awk -v v="$version" '
  $0 == "## " v { on = 1; next }
  on && /^## /   { exit }
  on             { print }
' "$root/CHANGELOG.md" | sed -e '1{/^$/d}'
