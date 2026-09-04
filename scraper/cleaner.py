"""
cleaner.py - מודול לניקוי והמרת HTML מפוסטים בפורום ימות המשיח ל-Markdown עשיר ומובנה.

מזהה וממיר:
- הדגשות (strong / b -> **text**)
- נטוי וקו תחתון (em / i -> *text*, u -> _text_)
- כותרות (h1..h6 -> #..#####)
- בלוקי קוד והגדרות ini (pre / code -> ```ini ...)
- רשימות (ul/ol/li -> - / 1. ...)
- טבלאות (table -> Markdown table)
- קווי הפרדה (hr -> ---)
- קישורים חכמים (a -> link_resolver / Markdown link)
- מסנן תגיות זבל, עוגני ניווט וברכות פורום ("ב"ה", "בס"ד", "שלום לכולם", "בהצלחה").
"""

import re
import html
from bs4 import BeautifulSoup

def make_slug(text: str) -> str:
    """יצירת slug נקי לעוגנים בעברית ובאנגלית"""
    if not text:
        return ""
    # הסרת תווים מיוחדים והמרה למקפים
    s = re.sub(r'[\s/\\|:?*\"\'<>()\[\].,!#~`]+', '-', text).strip('-')
    s = re.sub(r'-+', '-', s)
    return s

def extract_post_heading(html_content: str, post_index: int = 0, topic_title: str = "") -> str:
    """
    חילוץ כותרת הפוסט / הסעיף מתוך תוכן ה-HTML של הפוסט.
    """
    if not html_content or not html_content.strip():
        return topic_title if post_index == 0 else f"סעיף {post_index + 1}"

    soup = BeautifulSoup(html_content, 'html.parser')

    # 1. בדיקת תגיות כותרת h1..h6
    for h in soup.find_all(['h1', 'h2', 'h3', 'h4', 'h5', 'h6']):
        t = h.get_text(strip=True)
        if t and len(t) < 100:
            return t

    # 2. בדיקת טקסט מודגש בפסקה הראשונה
    first_p = soup.find('p')
    if first_p:
        first_b = first_p.find(['strong', 'b'])
        if first_b:
            bt = first_b.get_text(strip=True)
            if bt and 3 < len(bt) < 80:
                return bt

    # 3. בדיקת השורה הראשונה בטקסט
    text = soup.get_text(strip=True)
    lines = [l.strip() for l in text.splitlines() if l.strip()]
    if lines:
        l0 = lines[0]
        # ניקוי ברכות נפוצות
        for g in ['ב"ה', 'בס"ד', 'בס״ד', 'בעזהי"ת']:
            l0 = l0.replace(g, '').strip()
        if l0 and len(l0) < 60 and not l0.endswith('.'):
            return l0
        if l0 and any(l0.startswith(prefix) for prefix in [
            'הסבר', 'הגדרות', 'מודול', 'שלוחה', 'שלב', 'חלק',
            '1.', '2.', '3.', '4.', '5.', '6.', '7.', '8.', '9.', '10.'
        ]):
            return l0[:60]

    if post_index == 0:
        return topic_title
    return f"סעיף {post_index + 1}"

def clean_html_content(raw_html: str, link_resolver=None, current_file: str = "") -> str:
    """
    ממיר תוכן HTML של פוסט ל-Markdown עשיר, קריא וצלול למודלי AI.
    תומך בהדגשות, טבלאות, בלוקי קוד ופענוח קישורים חכם.
    """
    if not raw_html or not raw_html.strip():
        return ""

    soup = BeautifulSoup(raw_html, 'html.parser')

    # 1. הסרת תגיות מיותרות
    for tag in soup.find_all(['script', 'style', 'iframe', 'object']):
        tag.decompose()

    for a in soup.find_all('a', class_='anchor-offset'):
        a.decompose()

    for img in soup.find_all('img'):
        img.decompose()

    # הסרת ציטוטי תגובות למשתמשים אחרים
    for bq in soup.find_all('blockquote'):
        cite = bq.find(['cite', 'div'], class_=['user', 'quote-user'])
        if cite:
            bq.decompose()

    # 2. טיפול באלמנטים פנימיים (Inline Elements)
    # 2א. בלוקי קוד מוטבעים <code> (שאינם בתוך <pre>)
    for code in soup.find_all('code'):
        if code.parent and code.parent.name == 'pre':
            continue
        c_text = code.get_text()
        if c_text.strip():
            code.replace_with(f"`{c_text.strip()}`")
        else:
            code.decompose()

    # 2ב. קישורים <a>
    for a in soup.find_all('a'):
        href = a.get('href', '').strip()
        a_text = a.get_text().strip()
        if not href or not a_text:
            if a_text:
                a.replace_with(a_text)
            else:
                a.decompose()
            continue

        if link_resolver:
            md_link = link_resolver(href, a_text, current_file)
            a.replace_with(md_link)
        else:
            a.replace_with(f"[{a_text}]({href})")

    # 2ג. הדגשות <strong> ו-<b>
    for b in soup.find_all(['strong', 'b']):
        b_text = b.get_text()
        if b_text.strip():
            lead_ws = " " if b_text.startswith((' ', '\t', '\n')) else ""
            trail_ws = " " if b_text.endswith((' ', '\t', '\n')) else ""
            b.replace_with(f"{lead_ws}**{b_text.strip()}**{trail_ws}")
        else:
            b.decompose()

    # 2ד. נטוי <em> ו-<i>
    for i in soup.find_all(['em', 'i']):
        i_text = i.get_text()
        if i_text.strip():
            lead_ws = " " if i_text.startswith((' ', '\t', '\n')) else ""
            trail_ws = " " if i_text.endswith((' ', '\t', '\n')) else ""
            i.replace_with(f"{lead_ws}*{i_text.strip()}*{trail_ws}")
        else:
            i.decompose()

    # 2ה. קו תחתון <u>
    for u in soup.find_all('u'):
        u_text = u.get_text()
        if u_text.strip():
            lead_ws = " " if u_text.startswith((' ', '\t', '\n')) else ""
            trail_ws = " " if u_text.endswith((' ', '\t', '\n')) else ""
            u.replace_with(f"{lead_ws}_{u_text.strip()}_{trail_ws}")
        else:
            u.decompose()

    # 2ו. קו חוצה <del>, <s>, <strike>
    for s in soup.find_all(['del', 's', 'strike']):
        s_text = s.get_text()
        if s_text.strip():
            s.replace_with(f"~~{s_text.strip()}~~")
        else:
            s.decompose()

    # 3. טיפול באלמנטי בלוק (Block Elements)
    # 3א. בלוקי קוד <pre> (הגדרות ini)
    for pre in soup.find_all('pre'):
        code = pre.find('code')
        code_text = code.get_text() if code else pre.get_text()
        code_text = code_text.strip()
        pre.replace_with(f"\n\n```ini\n{code_text}\n```\n\n")

    # 3ב. טבלאות <table>
    for table in soup.find_all('table'):
        rows = []
        for tr in table.find_all('tr'):
            cells = [td.get_text().strip().replace('\n', ' ') for td in tr.find_all(['td', 'th'])]
            if any(cells):
                rows.append(cells)
        if rows:
            col_count = max(len(r) for r in rows)
            for r in rows:
                while len(r) < col_count:
                    r.append("")
            header = "| " + " | ".join(rows[0]) + " |"
            divider = "| " + " | ".join(["---"] * col_count) + " |"
            body = ["| " + " | ".join(r) + " |" for r in rows[1:]]
            table_md = "\n\n" + "\n".join([header, divider] + body) + "\n\n"
            table.replace_with(table_md)
        else:
            table.decompose()

    # 3ג. רשימות <ul> ו-<ol>
    for ul in soup.find_all('ul'):
        items = []
        for li in ul.find_all('li', recursive=False):
            li_text = li.get_text().strip()
            if li_text:
                items.append(f"- {li_text}")
        if items:
            ul.replace_with("\n" + "\n".join(items) + "\n")
        else:
            ul.decompose()

    for ol in soup.find_all('ol'):
        items = []
        for idx, li in enumerate(ol.find_all('li', recursive=False), 1):
            li_text = li.get_text().strip()
            if li_text:
                items.append(f"{idx}. {li_text}")
        if items:
            ol.replace_with("\n" + "\n".join(items) + "\n")
        else:
            ol.decompose()

    # 3ד. כותרות <h1>..<h6>
    for level in range(1, 7):
        for h in soup.find_all(f'h{level}'):
            h_text = h.get_text().strip()
            hashes = '#' * min(level + 1, 5)
            if h_text:
                h.replace_with(f"\n\n{hashes} {h_text}\n\n")
            else:
                h.decompose()

    # 3ה. קווי הפרדה <hr>
    for hr in soup.find_all('hr'):
        hr.replace_with('\n\n---\n\n')

    # 3ו. ירידות שורה <br>
    for br in soup.find_all('br'):
        br.replace_with('\n')

    # 3ז. פסקאות <p>
    for p in soup.find_all('p'):
        p_text = p.get_text().strip()
        if p_text:
            p.replace_with(f"\n{p_text}\n")
        else:
            p.decompose()

    # 3ח. ציטוטים נותרים <blockquote>
    for bq in soup.find_all('blockquote'):
        bq_text = bq.get_text().strip()
        if bq_text:
            lines = [f"> {line}" for line in bq_text.splitlines() if line.strip()]
            bq.replace_with("\n\n" + "\n".join(lines) + "\n\n")
        else:
            bq.decompose()

    # המרה לטקסט
    raw_text = soup.get_text()
    raw_text = html.unescape(raw_text)

    # 4. סינון שורות ברכה ופתיחה/סיום של פורומים
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
    for line in raw_text.splitlines():
        trimmed = line.strip()
        is_junk = False
        for pat in junk_patterns:
            if re.match(pat, trimmed):
                is_junk = True
                break
        if not is_junk:
            lines.append(line.rstrip())

    text = "\n".join(lines)
    text = re.sub(r'\n{3,}', '\n\n', text)
    return text.strip()
