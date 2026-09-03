#!/usr/bin/env bash
# פורס את gas/ai_yemot.gs לפרויקט ה-Apps Script ומעדכן את ה-deployment
# הקיים באותה כתובת /exec. מריצים אותו הוורקר deploy-gas.yml בכל שינוי.
#
# סודות נדרשים:
#   CLASPRC_JSON      — תוכן ~/.clasprc.json מ-`clasp login` במחשב מקומי
#   GAS_SCRIPT_ID     — Project Settings → Script ID
#   GAS_DEPLOYMENT_ID — (רשות) ה-deployment שמגיש את /exec. אם חסר, נבחר
#                       אוטומטית כשיש בדיוק אחד; אחרת נעצרים במקום לייצר
#                       כתובת חדשה בשקט.
set -euo pipefail
: "${CLASPRC_JSON:?חסר הסוד CLASPRC_JSON}"
: "${GAS_SCRIPT_ID:?חסר הסוד GAS_SCRIPT_ID}"

SRC="${SRC:-gas/ai_yemot.gs}"
[ -s "$SRC" ] || { echo "❌ $SRC לא נמצא"; exit 1; }

# גרסה מקובעת: clasp 3 שינה את מבנה קובץ-ההרשאות, וטוקן שנוצר ב-2.x
# אינו נקרא שם. שינוי הגרסה כאן מחייב יצירת טוקן מחדש.
npm install -g @google/clasp@2.4.2 >/dev/null 2>&1
printf '%s' "$CLASPRC_JSON" > "$HOME/.clasprc.json"
mkdir -p work
printf '{"scriptId":"%s","rootDir":"work"}\n' "$GAS_SCRIPT_ID" > .clasp.json

# מורידים קודם את המצב המרוחק: כך נשמר ה-manifest של הפרויקט (הרשאות,
# אזור-זמן, הגדרות ה-webapp) ולא נדרס בגרסה מומצאת, וגם יש גיבוי למה
# שהיה שם לפני הדריסה.
echo "== שולף את המצב המרוחק =="
clasp pull || { echo "❌ clasp pull נכשל — בדוק ש-Apps Script API מאופשר"; exit 1; }
ls -la work/
mkdir -p remote_backup && cp -a work/. remote_backup/ 2>/dev/null || true

# חייבים לכתוב לאותו שם-קובץ שקיים מרחוק: הוספת שם חדש הייתה משאירה שני
# קבצים עם אותן פונקציות — התנגשות הצהרות, והסקריפט מפסיק לעבוד.
TARGET=""
COUNT=$(find work -maxdepth 1 -type f \( -name '*.gs' -o -name '*.js' \) | wc -l | tr -d ' ')
if [ "$COUNT" = "1" ]; then
  TARGET=$(find work -maxdepth 1 -type f \( -name '*.gs' -o -name '*.js' \))
elif [ "$COUNT" = "0" ]; then
  TARGET="work/Code.gs"
else
  TARGET=$(find work -maxdepth 1 -type f -name "$(basename "$SRC")" | head -1)
  [ -n "$TARGET" ] || { echo "❌ יש $COUNT קבצי-קוד מרחוק ואין התאמה לשם $(basename "$SRC")."
                        echo "   קבע SRC לשם הנכון, או אחד את הקבצים בעורך."; exit 1; }
fi

# --- בחירת ה-deployment לעדכון (לפני הדריסה: השער נשען עליו) ---
# clasp deploy בלי -i יוצר כתובת חדשה, ואז ה-/exec הקיים ממשיך להגיש
# קוד ישן — לכן לא עושים זאת אוטומטית.
clasp deployments > deps.txt 2>&1 || true
cat deps.txt
DEP="${GAS_DEPLOYMENT_ID:-}"
if [ -z "$DEP" ]; then
  # שורות בצורה: - <deploymentId> @<version|HEAD> - <תיאור>
  DEPS=$(grep -oE '^- [A-Za-z0-9_-]{20,} @[0-9]+' deps.txt | awk '{print $2}' | sort -u || true)
  N=$(printf '%s\n' "$DEPS" | grep -c . || true)
  if [ "$N" = "1" ]; then
    DEP="$DEPS"; echo "נבחר deployment יחיד: $DEP"
  else
    echo "❌ נמצאו $N deployments מגורסאות. הגדר את הסוד GAS_DEPLOYMENT_ID"
    echo "   כדי לא ליצור כתובת /exec חדשה בטעות."
    exit 1
  fi
fi

# --- שער-בטיחות: מזהה-פרויקט שגוי הוא טעות-הקלדה של שורה אחת, והתוצאה היא
# דריסת סקריפט אחר לגמרי והפצתו לכתובת שלו.
SRC_MARK="${SRC_MARK:-AI_yemot}"
SIG_OK=0
[ -s "$TARGET" ] || SIG_OK=1                            # פרויקט ריק
grep -q "$SRC_MARK" "$TARGET" 2>/dev/null && SIG_OK=1   # HEAD הוא שלנו
if [ "$SIG_OK" = "0" ]; then
  VER=$(grep -F "$DEP" deps.txt | grep -oE '@[0-9]+' | tr -d '@' | head -1 || true)
  if [ -n "${VER:-}" ]; then
    echo "HEAD אינו נושא את החתימה — בודק את הגרסה החיה ($VER)"
    mkdir -p vercheck
    printf '{"scriptId":"%s","rootDir":"vercheck"}\n' "$GAS_SCRIPT_ID" > .clasp.json
    clasp pull --versionNumber "$VER" >/dev/null 2>&1 || true
    printf '{"scriptId":"%s","rootDir":"work"}\n' "$GAS_SCRIPT_ID" > .clasp.json
    grep -rqF "$SRC_MARK" vercheck 2>/dev/null && SIG_OK=1
  fi
fi

if [ "$SIG_OK" = "0" ]; then
  if [ "${ALLOW_UNSAFE_OVERWRITE:-false}" = "true" ] || [ "${SRC_MARK:-}" = "FORCE" ]; then
    echo "⚠️ עקיפת שער בטיחות פעילה (FORCE) - ממשיך בדריסה."
    SIG_OK=1
  else
    echo "❌ עצירה: לא ב-HEAD ולא בגרסה החיה נמצאה החתימה '$SRC_MARK' —"
    echo "   כלומר GAS_SCRIPT_ID מצביע על פרויקט אחר, או שעדיין לא נוספה לו החתימה."
    echo "   כדי לאשר דריסה ראשונית של הפרויקט, הפעל את הוורקר ידנית (workflow_dispatch) עם force=true."
    exit 1
  fi
fi

echo "== כותב ל-$TARGET =="
cp "$SRC" "$TARGET"

echo "== דוחף =="
clasp push -f

DESC="auto ${GITHUB_SHA:-local}"
echo "== מעדכן deployment $DEP =="
clasp deploy -i "$DEP" -d "$DESC"

{
  echo "## ✅ סקריפט Apps Script נפרס"
  echo ""
  echo "| שדה | ערך |"
  echo "|---|---|"
  echo "| קובץ-מקור | \`$SRC\` |"
  echo "| נכתב כ- | \`$(basename "$TARGET")\` |"
  echo "| deployment | \`$DEP\` (אותה כתובת /exec) |"
  echo "| תיאור | $DESC |"
  echo ""
  echo "גיבוי המצב המרוחק שלפני הדריסה מצורף כתוצר \`gas-remote-backup\`."
} >> "${GITHUB_STEP_SUMMARY:-/dev/stdout}"

echo "DONE deployment=$DEP"