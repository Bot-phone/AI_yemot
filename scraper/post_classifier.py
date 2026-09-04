"""
post_classifier.py - מודול לסיווג והכרעה על פוסטים מפורום ימות המשיח
מכריע לכל פוסט בנפרד האם הוא מכיל תיעוד/הגדרות טכניות (INCLUDE)
או שמדובר בשאלות, תלונות, ספאם או דיונים (EXCLUDE), ללא תלות בזהות הכותב.
"""

import re
import html
from cleaner import clean_html_content

# ביטויים רגולריים לזיהוי שורות הגדרה (key=value)
INI_SETTING_REGEX = re.compile(r'^[a-zA-Z0-9_\.\-]+=[^\n]+', re.MULTILINE)

# ביטויים של הגדרות מערכת אופייניות לימות המשיח
YEMOT_KEYWORDS = [
    'type=', 'digits=', 'ivr2:', 'ivr.ini', 'ext.ini', 'M1000', 'enter_id',
    'access_filter', 'recording', 'playfile', 'confbridge', 'routing',
    'say_name', 'check_did', 'downloadfile', 'uploadfile', 'ymgr',
    'הגדרה:', 'הגדרות:', 'ערך ברירת מחדל:', 'ברירת מחדל:', 'הודעות קשורות:',
    'הודעה ראשונה:', 'שלוחה 1:', 'שלוחה 2:', 'פרמטרים:', 'תבנית הקובץ:'
]

# ביטויים המעידים על שאלות משתמשים, דיונים ותלונות (EXCLUDE)
QUESTION_CHATTER_PATTERNS = [
    r'^\s*איך\s+(מגדירים|עושים|אפשר|פותחים|יוצרים|שולחים)',
    r'^\s*האם\s+(אפשר|יש|ניתן|מישהו)',
    r'^\s*מישהו\s+יודע',
    r'^\s*למה\s+(זה\s+לא|אין|הוא)',
    r'^\s*שאלה\s*[:\?]',
    r'^\s*שאלה\s+דחופה',
    r'^\s*עזרה\s*[:!]',
    r'^\s*אשמח\s+לעזרה',
    r'^\s*לא\s+הבנתי',
    r'^\s*זה\s+לא\s+עובד',
    r'^\s*יש\s+לי\s+בעיה',
    r'^\s*יש\s+באג',
    r'^\s*תודה\s+(רבה|על|מראש)',
    r'^\s*יישר\s+כח',
    r'^\s*כל\s+הכבוד',
    r'^\s*מצטרף\s+(לבקשה|לשאלה)',
    r'^\s*אצלי\s+זה\s+לא\s+עובד',
    r'^\s*קופץ\s+לי\s+שגיאה',
    r'^\s*למישהו\s+יש\s+פתרון'
]

def classify_post(raw_html: str, cleaned_text: str, pid: int = None, post_filter: dict = None) -> tuple[bool, str]:
    """
    מכריע האם פוסט הוא תיעוד טכני (True) או זבל/שאלות/דיון (False).
    מחזיר (האם לכלול, סיבת ההכרעה).
    """
    post_filter = post_filter or {}
    include_pids = set(post_filter.get("include_pids", []))
    exclude_pids = set(post_filter.get("exclude_pids", []))

    # 1. בדיקת החרגה / הכללה מפורשת לפי PID
    if pid is not None:
        if pid in exclude_pids:
            return False, f"PID {pid} מוחרג מפורשות בהגדרות הנושא"
        if pid in include_pids:
            return True, f"PID {pid} נכלל מפורשות בהגדרות הנושא"

    if not cleaned_text or len(cleaned_text.strip()) < 25:
        return False, "פוסט קצר מדי (פחות מ-25 תווים)"

    # 2. בדיקה האם הפוסט הוא בעיקרו ציטוט של משתמש אחר ללא תוכן מקורי
    if '<blockquote' in raw_html.lower() and len(cleaned_text) < 80:
        return False, "הפוסט מכיל בעיקר ציטוט ללא הסבר טכני מספק"

    # 3. בדיקת דפוסי שאלות ושיח פורומים
    lines = [l.strip() for l in cleaned_text.splitlines() if l.strip()]
    first_few_lines = " ".join(lines[:3])

    for pat in QUESTION_CHATTER_PATTERNS:
        if re.search(pat, first_few_lines):
            # אם יש שאלת משתמש, נבדוק האם יש בה בכל זאת בלוק הגדרות מלא
            ini_matches = INI_SETTING_REGEX.findall(cleaned_text)
            if not ini_matches or len(ini_matches) < 2:
                return False, f"זוהה דפוס שאלת משתמש/דיון: {pat}"

    # 4. בדיקת סימני שאלה מרובים בפוסט קצר (מעיד על שאלת משתמש)
    if cleaned_text.count('?') >= 2 and len(cleaned_text) < 250:
        ini_matches = INI_SETTING_REGEX.findall(cleaned_text)
        if not ini_matches:
            return False, "הפוסט מכיל שאלות משתמש ללא הגדרות"

    # 5. בדיקת נוכחות הגדרות טכניות (אינדיקציה חזקה מאוד לתיעוד)
    ini_matches = INI_SETTING_REGEX.findall(cleaned_text)
    if len(ini_matches) >= 1:
        return True, f"מכיל {len(ini_matches)} הגדרות ini"

    for kw in YEMOT_KEYWORDS:
        if kw.lower() in cleaned_text.lower():
            return True, f"מכיל מילת מפתח טכנית: {kw}"

    # 6. בדיקת פסקאות הסבר מפורטות
    # אם הפוסט כולל הסבר ארוך ומובנה (מעל 150 תווים) וללא שאלות
    if len(cleaned_text) >= 120 and '?' not in cleaned_text:
        return True, "טקסט הסבר מפורט ללא שאלות"

    # ברירת מחדל: אם הפוסט לא עונה על אף קריטריון תיעודי
    return False, "אין אינדיקציה טכנית מספקת בפוסט"
