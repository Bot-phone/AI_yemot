"""
cleaner.py - מודול לניקוי ועיבוד HTML מפוסטים בפורום ימות המשיח
ממיר תוכן HTML לטקסט נקי, קריא וצלול ללא זבל פורומים.
"""

import re
import html
from bs4 import BeautifulSoup, NavigableString, Tag

def clean_html_content(raw_html: str) -> str:
    """
    מנקה תוכן HTML של פוסט מהפורום ומחזיר טקסט נקי לחלוטין.
    """
    if not raw_html or not raw_html.strip():
        return ""

    soup = BeautifulSoup(raw_html, 'html.parser')

    # הסרת אלמנטים מיותרים לחלוטין
    for tag in soup.find_all(['script', 'style', 'iframe', 'object']):
        tag.decompose()

    # הסרת עוגני ניווט פנימיים של NodeBB
    for a in soup.find_all('a', class_='anchor-offset'):
        a.decompose()

    # הסרת תמונות אמוג'י או תמונות זעירות (אמוג'י בפורום מופיעים לפעמים כ-img)
    for img in soup.find_all('img'):
        img.decompose()

    # טיפול בציטוטים: בפוסטים של ימות המשיח, לפעמים כותרות או דוגמאות עטופות ב-blockquote.
    # אם הציטוט הוא תגובה למשתמש אחר (מכיל post-header או ציטוט משתמש), נסיר אותו.
    for bq in soup.find_all('blockquote'):
        cite = bq.find(['cite', 'div'], class_=['user', 'quote-user'])
        if cite:
            bq.decompose()

    # טיפול בירידות שורה
    for br in soup.find_all('br'):
        br.replace_with('\n')

    # טיפול בכותרות
    for h in soup.find_all(['h1', 'h2', 'h3', 'h4', 'h5', 'h6']):
        text = h.get_text().strip()
        if text:
            h.replace_with(f"\n\n{text}\n")
        else:
            h.decompose()

    # טיפול בבלוקי קוד (הגדרות ini)
    for pre in soup.find_all('pre'):
        code = pre.find('code')
        code_text = code.get_text() if code else pre.get_text()
        pre.replace_with(f"\n\n{code_text.strip()}\n\n")

    # טיפול ברשימות מסודרות ולא מסודרות
    for ul in soup.find_all('ul'):
        for li in ul.find_all('li', recursive=False):
            li_text = li.get_text().strip()
            li.replace_with(f"\n* {li_text}")

    for ol in soup.find_all('ol'):
        idx = 1
        for li in ol.find_all('li', recursive=False):
            li_text = li.get_text().strip()
            li.replace_with(f"\n{idx}. {li_text}")
            idx += 1

    # טיפול בטבלאות: המרה לטקסט מיושר ונקי
    for table in soup.find_all('table'):
        rows = []
        for tr in table.find_all('tr'):
            cells = [td.get_text().strip() for td in tr.find_all(['td', 'th'])]
            if any(cells):
                rows.append(" | ".join(cells))
        if rows:
            table.replace_with("\n\n" + "\n".join(rows) + "\n\n")
        else:
            table.decompose()

    # טיפול בפסקאות
    for p in soup.find_all('p'):
        text = p.get_text().strip()
        if text:
            p.replace_with(f"\n{text}\n")
        else:
            p.decompose()

    # המרה לטקסט גולמי
    text = soup.get_text()

    # פענוח ישויות HTML נוספות
    text = html.unescape(text)

    # סינון תבניות זבל של פורומים
    junk_patterns = [
        r'^\s*ב"ה\s*$',
        r'^\s*בס"ד\s*$',
        r'^\s*בס״ד\s*$',
        r'^\s*בעזהי"ת\s*$',
        r'^\s*שלום לכולם[.,!]*\s*$',
        r'^\s*שלום רב[.,!]*\s*$',
        r'^\s*בהצלחה[.,!]*\s*$',
        r'^\s*בברכה[.,!]*\s*$',
        r'^\s*יישר כח[.,!]*\s*$',
        r'^\s*המשך יבוא[.…]*\s*$',
        r'^\s*עד כאן בינתיים[.…]*\s*$',
        r'^\s*לשאלות והערות לחץ כאן\s*$',
        r'^\s*למעבר לשרשור התגובות לחץ כאן\s*$'
    ]

    lines = []
    for line in text.splitlines():
        trimmed = line.strip()
        is_junk = False
        for pat in junk_patterns:
            if re.match(pat, trimmed):
                is_junk = True
                break
        if not is_junk:
            lines.append(line.rstrip())

    text = "\n".join(lines)

    # נרמול ריווח ושורות ריקות עוקבות (מקסימום 2 ירידות שורה רצופות)
    text = re.sub(r'\n{3,}', '\n\n', text)
    
    return text.strip()
