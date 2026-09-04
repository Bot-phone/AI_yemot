"""
post_classifier.py - מודול לסיווג והכרעה על פוסטים מפורום ימות המשיח (קטגוריה 1 - תיעוד טכני)

מאחר וקטגוריה 1 סגורה לכתיבה של משתמשים רגילים ומשמשת לתיעוד רשמי והכרזות,
העיקרון המנחה הוא: **ברירת מחדל של הכללה (Include by default)**.
עדיף שייכנס פוסט קטן של רעש מאשר שיידלג תוכן טכני, עדכון או הסבר חשוב.
מסננים אך ורק רעש מובהק (פוסטים שנמחקו, פוסטים קצרים של ברכות/תודה ללא תוכן, או החרגות מפורשות).
"""

import re
import html
from cleaner import clean_html_content

# ביטויי מחיקה מובנים בפורום NodeBB
DELETED_PATTERNS = [
    '[[topic:post-is-deleted]]',
    'פוסט זה נמחק'
]

# ביטויים רגולריים לזיהוי שורות הגדרה (key=value)
INI_SETTING_REGEX = re.compile(r'^[a-zA-Z0-9_\.\-]+=[^\n]+', re.MULTILINE)

# דפוסי שיח וברכות קצרות ללא תוכן טכני
PURE_CHATTER_PATTERNS = [
    r'^(תודה|תודה רבה|תודה לכולם|יישר כח|ישר כח|כל הכבוד|מעולה|אשריך|מצטרף לבקשה|בהצלחה)\s*[!.]*$',
    r'^(איך\s+(מגדירים|עושים|אפשר|פותחים)|מישהו יודע|למה זה לא עובד|אשמח לעזרה|יש לי בעיה)\s*[\?!.]*$'
]

def classify_post(raw_html: str, cleaned_text: str, pid: int = None, post_filter: dict = None) -> tuple[bool, str]:
    """
    מכריע האם פוסט נכלל בתיעוד.
    ברירת המחדל היא הכללה (True).
    סינון מתבצע אך ורק עבור רעש מובהק, מחיקות או החרגות ידניות.
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

    # 2. סינון פוסטים שנמחקו
    cleaned_lower = cleaned_text.lower()
    for dp in DELETED_PATTERNS:
        if dp in cleaned_lower:
            return False, f"פוסט שנמחק בפורום ({dp})"

    stripped = cleaned_text.strip()

    # 3. סינון פוסט ריק או קצרצר במיוחד (פחות מ-15 תווים)
    if not stripped or len(stripped) < 15:
        return False, "פוסט קצר מדי (פחות מ-15 תווים)"

    # 4. בדיקת פוסט קצר שהוא אך ורק שאלת סרק / ברכה ללא שום הגדרות
    if len(stripped) < 100:
        has_ini = bool(INI_SETTING_REGEX.search(stripped))
        if not has_ini:
            for pat in PURE_CHATTER_PATTERNS:
                if re.match(pat, stripped, re.IGNORECASE):
                    return False, f"זוהה שיח/ברכה ללא תוכן תיעודי: {stripped}"

    # ברירת מחדל: כל פוסט בקטגוריית התיעוד מאושר להכללה
    return True, "הכללה (ברירת מחדל לקטגוריית תיעוד)"

