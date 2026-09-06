#!/usr/bin/env bash
# Compose the GitHub release body for one version:
#   1. direct download links for every installer (first, so nobody has to
#      scroll past the guide to the Assets box at the bottom of the page)
#   2. the Hebrew usage guide in desktop/RELEASE_GUIDE.md
#   3. "מה חדש" — the matching "## <version>" section of CHANGELOG.md
# Usage: .github/release-notes.sh <version> > body.md
# The asset names are what tauri-action uploads for productName "AI_yemot"
# with the bundle targets in .github/workflows/desktop.yml; if either changes,
# update the table below.
set -euo pipefail
version="${1:?version}"
root="$(cd "$(dirname "$0")/.." && pwd)"
repo="${GITHUB_REPOSITORY:-Bot-phone/AI_yemot}"
dl="https://github.com/${repo}/releases/download/v${version}"

exe="AI_yemot_${version}_x64-setup.exe"
portable="AI_yemot_${version}_x64-portable.exe"
msi="AI_yemot_${version}_x64_en-US.msi"
dmg="AI_yemot_${version}_aarch64.dmg"
appimage="AI_yemot_${version}_amd64.AppImage"
deb="AI_yemot_${version}_amd64.deb"

# GitHub keeps the `dir` attribute on a div, which is what makes the Hebrew
# guide render right-to-left; the English changelog goes back to LTR.
echo '<div dir="rtl">'
echo
echo '## הורדה'
echo
echo '| מערכת | קובץ להורדה |'
echo '|---|---|'
echo "| **Windows** | [${exe}](${dl}/${exe}) (מומלץ) · [${msi}](${dl}/${msi}) |"
echo "| **Windows — גרסה ניידת** | [${portable}](${dl}/${portable}) |"
echo "| **macOS** (Apple Silicon) | [${dmg}](${dl}/${dmg}) |"
echo "| **Linux** | [${appimage}](${dl}/${appimage}) · [${deb}](${dl}/${deb}) |"
echo
echo '**הגרסה הניידת** היא קובץ `.exe` יחיד שלא דורש התקנה: שומרים אותו בכל מקום (גם על דיסק-און-קי) ומפעילים. הוא מסתמך על רכיב WebView2 של מיקרוסופט, שמותקן כברירת מחדל ב-Windows 10 ו-11; אם ההפעלה נכשלת, מתקינים אותו פעם אחת מ-[הדף של מיקרוסופט](https://developer.microsoft.com/microsoft-edge/webview2/). הגרסה הניידת לא מתקינה עדכונים בעצמה: כשיוצאת גרסה חדשה היא מציעה להוריד את הקובץ החדש במקומה. פרטי ההתחברות נשמרים במנהל ההרשאות של Windows במחשב שבו מריצים, לא בתוך הקובץ.'
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
