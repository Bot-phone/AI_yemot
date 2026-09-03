# Google Apps Script Backend — AI_yemot

תיקייה זו מכילה את סקריפט ה-Apps Script (`ai_yemot.gs`) המשמש כצד-שרת עבור המערכת.

**המאגר הוא מקור-האמת.** כל דחיפה (push) שנוגעת בקבצים תחת `gas/**` או ב-`.github/workflows/deploy-gas.yml` מפעילה אוטומטית את הוורקר *Deploy Script (Apps Script)*. הוורקר בודק את תחביר הקוד, שולף את הפרויקט, דוחף את הקובץ ומעדכן את ה-deployment הקיים — **באותה כתובת `/exec`**, ללא צורך בהדבקה ידנית או יצירת גרסה חדשה בעורך של Google.

---

## הגדרה חד-פעמית

1. **הפעלת ה-Google Apps Script API**:
   - היכנס אל <https://script.google.com/home/usersettings>
   - העבר את **Google Apps Script API** למצב **On**.

2. **הפקת טוקן הרשאה (clasp login)**:
   - במחשב מקומי, הריצו:
     ```sh
     npx @google/clasp@2.4.2 login
     ```
   - ייפתח דפדפן לאישור הגישה בחשבון גוגל.
   - לאחר ההתחברות נוצר הקובץ `~/.clasprc.json` (ב-Windows: `C:\Users\<username>\.clasprc.json`).
   - העתיקו את **כל תוכן הקובץ** — זהו הסוד `CLASPRC_JSON`.

3. **שני המזהים הנדרשים**:
   - `GAS_SCRIPT_ID` — מתוך עורך הסקריפט: *הגדרות פרויקט (Project Settings)* ← **Script ID**.
   - `GAS_DEPLOYMENT_ID` — מתוך עורך הסקריפט: *פריסה (Deploy)* ← *ניהול פריסות (Manage deployments)* ← מזהה הפריסה שמגיש את ה-`/exec` (מחרוזת ארוכה שמתחילה לרוב ב-`AKfycb...`).

4. **הגדרת סודות בגיטהאב**:
   בהגדרות המאגר: **Settings ← Secrets and variables ← Actions ← New repository secret**:

   | שם הסוד | חובה | תיאור |
   |---|---|---|
   | `CLASPRC_JSON` | ✅ חובה | תוכן הקובץ `~/.clasprc.json` |
   | `GAS_SCRIPT_ID` | ✅ חובה | ה-Script ID של פרויקט ה-Apps Script |
   | `GAS_DEPLOYMENT_ID` | מומלץ מאוד | ה-Deployment ID הקיים, כדי לעדכן אותו במקום לייצר כתובת `/exec` חדשה |

---

## שער בטיחות

הוורקר כולל שער בטיחות כדי למנוע דריסה בטעות של פרויקט אחר במקרה של טעות ב-`GAS_SCRIPT_ID`:
הוא מוודא שהפרויקט המרוחק ריק או מכיל את החתימה `AI_yemot`.
בפריסה ראשונית לפרויקט קיים שלא נבנה במקור דרך המאגר, ניתן להפעיל את הוורקר ידנית בלשונית **Actions** דרך **Run workflow** ולסמן את אפשרות ה-`force` (עקיפת שער בטיחות).