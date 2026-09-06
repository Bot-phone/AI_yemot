#!/usr/bin/env bash
# Compose the GitHub release body for one version:
#   1. the Hebrew usage guide in desktop/RELEASE_GUIDE.md (every release, first)
#   2. "מה חדש" — the matching "## <version>" section of CHANGELOG.md
# Usage: .github/release-notes.sh <version> > body.md
set -euo pipefail
version="${1:?version}"
root="$(cd "$(dirname "$0")/.." && pwd)"

# GitHub keeps the `dir` attribute on a div, which is what makes the Hebrew
# guide render right-to-left; the English changelog goes back to LTR.
echo '<div dir="rtl">'
echo
cat "$root/desktop/RELEASE_GUIDE.md"
echo
echo "## מה חדש בגרסה ${version}"
echo
echo '</div>'
echo
echo '<div dir="ltr">'
echo
# Portable (BSD awk on macOS runners too): print the section, dropping the
# blank line that follows the heading.
awk -v v="$version" '
  $0 == "## " v { on = 1; first = 1; next }
  on && /^## /   { exit }
  on             { if (first && $0 == "") { first = 0; next } first = 0; print }
' "$root/CHANGELOG.md"
echo
echo '</div>'
