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
            clean_href = href.strip()
            if " " in clean_href and not (clean_href.startswith("<") and clean_href.endswith(">")):
                clean_href = f"<{clean_href}>"
            a.replace_with(f"[{a_text}]({clean_href})")

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


# ============================================================================
# Post-processing של קובצי הידע (knowledge/*.txt)
# ----------------------------------------------------------------------------
# פונקציה אחת, אידמפוטנטית, שמופעלת גם על הקבצים הקיימים (scraper/postprocess.py)
# וגם בזמן הכתיבה בסנכרון (sync_docs.py), כדי שהקורפוס יישאר נקי גם בעתיד.
# ============================================================================

# הקובץ שבו נשמר בלוק "התוספים הניתנים להגדרה בכל מודול" פעם אחת בלבד.
ADDONS_CANONICAL_FILE = "סדר פעולות בכניסה לשלוחה לפני המעבר למודול עצמו.txt"
ADDONS_TITLE = "תוספים הניתנים להגדרה בכל מודול"
ADDONS_ANCHOR = "תוספים-הניתנים-להגדרה-בכל-מודול"
ADDONS_POINTER = f"> ראו: [{ADDONS_TITLE}](<{ADDONS_CANONICAL_FILE}#{ADDONS_ANCHOR}>)"

# הכותרת החוזרת שהופיעה בעשרות קבצים (##### **קישורים לתוספים...**)
_ADDONS_HEADER_RE = re.compile(
    r'^\s{0,3}#{3,6}\s*\**\s*קישורים לתוספים שניתנים להגדרה בכל מודול'
)

# שורות הציטוט הסטנדרטיות של אותו בלוק - אלו שנמחקות.
# שורות ציטוט אחרות (הערות ייחודיות למודול) נשמרות.
_ADDONS_STD_LINE_PATTERNS = (
    "סדר פעולות בכניסה לשלוחה לפני המעבר למודול עצמו.txt#",
    "הגדרות הזיהוי בכלל המערכת.txt#כניסה-לפי-מספר-אישי",
    "והסברים כלליים.txt#הודעת-ברוכים-הבאים",
    "הרשאות כניסה לשלוחה.txt#להלן-ההגדרות-שקשורות-להרשאות-כניסה-לשלוחה",
)

ADDONS_CANONICAL_BLOCK = "\n".join([
    "---",
    "",
    '<a id="' + ADDONS_ANCHOR + '"></a>',
    "## " + ADDONS_TITLE,
    "",
    "התוספים הבאים ניתנים להגדרה בכל מודול במערכת. הבלוק מרוכז כאן פעם אחת, ושאר קובצי הידע מפנים לכאן.",
    "",
    "> ##### [רשימת כל ההגדרות שניתנות להטמעה בכל מודול](#סדר-פעולות-בכניסה-לשלוחה-לפני-המעבר-למודול-עצמו)",
    "> ##### [הגדרות זיהוי בכניסה לשלוחה](<הגדרות הזיהוי בכלל המערכת.txt#כניסה-לפי-מספר-אישי>)",
    "> **לתשומת לב:** הגדרת הזיהוי שתגדירו בשלוחה תשפיע גם על הגדרות אחרות (כגון פילטרים) בשלוחה אליה המשתמש יעבור.",
    "> ##### [הסבר על הודעת ברוכים הבאים](<הגדרות גלובליות-הגדרות לכל השלוחות-הגדרות בכניסה לשלוחות ולמערכת-והסברים כלליים.txt#הודעת-ברוכים-הבאים>) (הודעה ראשונה בשלוחה)",
    "> ##### [הרשאות כניסה לשלוחה](<הרשאות כניסה לשלוחה.txt#להלן-ההגדרות-שקשורות-להרשאות-כניסה-לשלוחה>)",
    "",
])

TYPES_CATALOG_FILE = "כל סוגי השלוחות הקיימות.txt"


def _fence_map(lines):
    """מחזיר רשימת בוליאנים - האם השורה נמצאת בתוך בלוק קוד (``` / ~~~)."""
    inside = []
    fence = None
    for line in lines:
        stripped = line.strip()
        m = re.match(r'^(`{3,}|~{3,})', stripped)
        if m:
            tok = m.group(1)[0]
            if fence is None:
                fence = tok
                inside.append(True)
                continue
            if tok == fence:
                inside.append(True)
                fence = None
                continue
        inside.append(fence is not None)
    return inside


# --- 1. פענוח קידוד אחוזים (%D7..) בתוך יעדי קישורים -------------------------

_PCT_RUN_RE = re.compile(r'(?:%[89A-Fa-f][0-9A-Fa-f])+')
_MD_LINK_RE = re.compile(r'(\]\()(<?)([^)\n]*?)(>?)(\))')


def _decode_pct_run(m):
    raw = m.group(0)
    try:
        data = bytes(int(raw[i + 1:i + 3], 16) for i in range(0, len(raw), 3))
        text = data.decode('utf-8')
    except Exception:
        return raw
    # רק תווים בטווח העברי - לא נוגעים בשאר (וכך גם לא נוצרים רווחים)
    if not text or not all('֐' <= ch <= '׿' for ch in text):
        return raw
    return text


def decode_percent_encoded_links(text: str) -> str:
    """מפענח רצפי %D7.. (עברית) בתוך יעדי קישורי Markdown, בלי לפגוע ב-%20 וכד'."""
    def repl(m):
        open_p, lt, target, gt, close_p = m.groups()
        new_target = _PCT_RUN_RE.sub(_decode_pct_run, target)
        # שמירה על המוסכמה: יעד עם רווחים חייב להיות עטוף ב-<...>
        if " " in new_target and not lt:
            lt, gt = "<", ">"
        return open_p + lt + new_target + gt + close_p

    return _MD_LINK_RE.sub(repl, text)


# --- 2. עוגנים: עוגן אחד לכל כותרת + מזהים ייחודיים --------------------------

_NAV_ANCHOR_RE = re.compile(r'<a id="(?:topic|post)-\d+"></a>')
_ANCHOR_ID_RE = re.compile(r'(<a id=")([^"]*)("></a>)')


def strip_nav_anchors(text: str) -> str:
    """הסרת עוגני הניווט הפנימיים topic-N / post-N (נשאר עוגן ה-slug בלבד)."""
    return _NAV_ANCHOR_RE.sub('', text)


def dedupe_anchor_ids(text: str) -> str:
    """הפיכת מזהי עוגנים כפולים לייחודיים (slug, slug-2, slug-3 ...)."""
    seen = set()
    counters = {}

    def repl(m):
        pre, anchor_id, post = m.groups()
        if anchor_id not in seen:
            seen.add(anchor_id)
            return m.group(0)
        base = re.sub(r'-\d+$', '', anchor_id)
        idx = counters.get(base, 1)
        while True:
            idx += 1
            candidate = base + '-' + str(idx)
            if candidate not in seen:
                break
        counters[base] = idx
        seen.add(candidate)
        return pre + candidate + post

    return _ANCHOR_ID_RE.sub(repl, text)


# --- 3. בלוק "קישורים לתוספים שניתנים להגדרה בכל מודול" ----------------------

def _is_std_addons_line(line: str) -> bool:
    return any(pat in line for pat in _ADDONS_STD_LINE_PATTERNS)


def dedupe_addons_block(text: str, filename: str = "") -> str:
    """
    משאיר את בלוק התוספים פעם אחת בקובץ הקנוני, ומחליף אותו בשאר הקבצים
    בשורת הפניה אחת. הערות ייחודיות למודול (שורות ציטוט שאינן סטנדרטיות) נשמרות.
    """
    lines = text.split('\n')
    inside = _fence_map(lines)
    out = []
    i = 0
    n = len(lines)
    while i < n:
        if inside[i] or not _ADDONS_HEADER_RE.match(lines[i]):
            out.append(lines[i])
            i += 1
            continue

        # איסוף גוש הציטוט שאחרי הכותרת
        j = i + 1
        quote_lines = []
        while j < n and not inside[j] and (not lines[j].strip() or lines[j].lstrip().startswith('>')):
            if lines[j].strip():
                quote_lines.append(lines[j].rstrip())
            j += 1

        if not any(_is_std_addons_line(q) for q in quote_lines):
            # לא בלוק הבוילרפלייט - משאירים כמו שהוא
            out.append(lines[i])
            i += 1
            continue

        custom = [q for q in quote_lines if not _is_std_addons_line(q)]
        out.append(ADDONS_POINTER)
        out.extend(custom)
        out.append("")
        i = j

    result = '\n'.join(out)

    if filename == ADDONS_CANONICAL_FILE and ('id="' + ADDONS_ANCHOR + '"') not in result:
        result = result.rstrip('\n') + '\n\n' + ADDONS_CANONICAL_BLOCK

    return result


# --- 4. הסרת עמודות ריקות לחלוטין מטבלאות גדולות -----------------------------

def _split_row(line: str):
    s = line.strip()
    if s.startswith('|'):
        s = s[1:]
    if s.endswith('|'):
        s = s[:-1]
    return [c.strip() for c in s.split('|')]


def _process_table_block(block, min_rows):
    rows = [_split_row(l) for l in block]
    if len(rows) < min_rows + 2:
        return block
    width = len(rows[0])
    if width < 3 or any(len(r) != width for r in rows):
        return block
    # שורה 1 חייבת להיות שורת מפריד
    if not all(re.fullmatch(r':?-{2,}:?', c) for c in rows[1]):
        return block
    body = rows[2:]
    keep = [c for c in range(width) if c < 2 or any(r[c] for r in body)]
    if len(keep) == width or len(keep) < 2:
        return block
    dropped = [rows[0][c] for c in range(width) if c not in keep]
    note = "> הערה: הוסרו מטבלה זו עמודות שהיו ריקות לחלוטין בכל השורות: " + ", ".join(
        d or "(ללא כותרת)" for d in dropped) + "."
    table = ["| " + " | ".join(r[c] for c in keep) + " |" for r in rows]
    return [note, ""] + table


def drop_empty_table_columns(text: str, min_rows: int = 20) -> str:
    """
    בטבלאות Markdown גדולות (min_rows+ שורות גוף) - הסרת עמודות שריקות לחלוטין
    בכל שורות הגוף (שתי העמודות הראשונות נשמרות תמיד).
    """
    lines = text.split('\n')
    inside = _fence_map(lines)
    out = []
    i = 0
    n = len(lines)
    while i < n:
        if inside[i] or not lines[i].lstrip().startswith('|'):
            out.append(lines[i])
            i += 1
            continue
        j = i
        while j < n and not inside[j] and lines[j].lstrip().startswith('|'):
            j += 1
        out.extend(_process_table_block(lines[i:j], min_rows))
        i = j
    return '\n'.join(out)


# --- 5. נרמול רשימת סוגי השלוחות --------------------------------------------

# יעד קישור: או עטוף ב-<...> או מחרוזת ללא רווחים וללא סוגריים סוגרים
_LINK_TARGET = r'(<[^>]*>|[^\s)]*)'
_TYPE_BULLET_NORMALIZED_RE = re.compile(
    r'^-\s*type=([A-Za-z0-9_]+)(?:\s+—\s+(.*?))?(?:\s+\(\[קובץ\]\(' + _LINK_TARGET + r'\)\))?\s*$'
)
_TYPE_BULLET_LINKED_RE = re.compile(
    r'^-\s*\[\s*([A-Za-z0-9_]+)\s*=\s*(.*?)\s*\]\(' + _LINK_TARGET + r'\)\s*(.*)$'
)
_TYPE_BULLET_PLAIN_RE = re.compile(r'^-\s*([A-Za-z0-9_]+)\s*=\s*(.*)$')


def normalize_type_catalog(text: str) -> str:
    """
    נרמול כל שורות סוגי השלוחות לצורה אחידה הניתנת לפירסור:
        - type=<slug> — <תיאור> ([קובץ](<link>))
    ה-slug נלקח מבלוק ה-ini הצמוד שאחרי השורה (מקור האמת).
    """
    lines = text.split('\n')
    inside = _fence_map(lines)

    def next_ini_type(start):
        for k in range(start + 1, min(start + 12, len(lines))):
            if not inside[k] and lines[k].lstrip().startswith('- '):
                return None
            m = re.match(r'^\s*type\s*=\s*([A-Za-z0-9_]+)\s*$', lines[k])
            if m and inside[k]:
                return m.group(1)
        return None

    out = list(lines)
    for idx, line in enumerate(lines):
        if inside[idx] or not line.lstrip().startswith('- '):
            continue

        slug = desc = link = note = None
        m = _TYPE_BULLET_NORMALIZED_RE.match(line)
        if m:
            slug, desc, link = m.group(1), m.group(2), m.group(3)
        else:
            m = _TYPE_BULLET_LINKED_RE.match(line)
            if m:
                slug, desc, link, note = m.group(1), m.group(2), m.group(3), m.group(4)
            else:
                m = _TYPE_BULLET_PLAIN_RE.match(line)
                if m:
                    slug, desc = m.group(1), m.group(2)
        if slug is None:
            continue

        ini_slug = next_ini_type(idx)
        if not ini_slug:
            continue
        slug = ini_slug

        desc = (desc or "").strip()
        if note:
            desc = (desc + " " + note.strip()).strip()
        new_line = "- type=" + slug + (" — " + desc if desc else "")
        if link:
            link = link.strip()
            if " " in link and not (link.startswith("<") and link.endswith(">")):
                link = "<" + link + ">"
            new_line += " ([קובץ](" + link + "))"
        out[idx] = new_line

    return '\n'.join(out)


# --- 6. ניקוי רווחים, שורות ריקות וקווי הפרדה עוקבים -------------------------

def tidy_whitespace(text: str) -> str:
    lines = text.split('\n')
    inside = _fence_map(lines)
    lines = [l if inside[i] else l.rstrip() for i, l in enumerate(lines)]

    # איחוד קווי הפרדה --- עוקבים (עם שורות ריקות ביניהם) לקו אחד
    out = []
    i = 0
    n = len(lines)
    while i < n:
        if not inside[i] and lines[i].strip() == '---':
            last = i
            j = i + 1
            while j < n and not inside[j] and (not lines[j].strip() or lines[j].strip() == '---'):
                if lines[j].strip() == '---':
                    last = j
                j += 1
            out.append('---')
            i = last + 1
            continue
        out.append(lines[i])
        i += 1

    text = '\n'.join(out)

    # כיווץ רצפי שורות ריקות מחוץ לבלוקי קוד
    lines = text.split('\n')
    inside = _fence_map(lines)
    out = []
    blanks = 0
    for i, l in enumerate(lines):
        if not inside[i] and not l.strip():
            blanks += 1
            if blanks > 1:
                continue
        else:
            blanks = 0
        out.append(l)
    return '\n'.join(out).strip() + '\n'


# --- הפונקציה הראשית ---------------------------------------------------------

def postprocess_knowledge_text(text: str, filename: str = "") -> str:
    """
    ניקוי-על אידמפוטנטי של קובץ ידע (knowledge/*.txt):
      1. פענוח קידוד אחוזים בעברית בתוך יעדי קישורים.
      2. עוגן אחד לכל כותרת (הסרת topic-N/post-N) + מזהים ייחודיים.
      3. איחוד בלוק "התוספים הניתנים להגדרה בכל מודול" לקובץ אחד + הפניות.
      4. הסרת עמודות ריקות לחלוטין מטבלאות גדולות.
      5. נרמול רשימת סוגי השלוחות לצורה `- type=<slug> — <תיאור>`.
      6. ניקוי רווחים, שורות ריקות מיותרות וקווי הפרדה עוקבים.
    """
    if not text:
        return text

    text = text.replace('\r\n', '\n').replace('\r', '\n')
    text = decode_percent_encoded_links(text)
    text = strip_nav_anchors(text)
    text = dedupe_addons_block(text, filename)
    text = drop_empty_table_columns(text)
    if filename == TYPES_CATALOG_FILE:
        text = normalize_type_catalog(text)
    text = dedupe_anchor_ids(text)
    text = tidy_whitespace(text)
    return text
