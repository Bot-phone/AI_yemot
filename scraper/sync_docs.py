#!/usr/bin/env python3
"""
sync_docs.py - וורקר לסנכרון, סיווג, ניקוי ושמירת תיעוד ימות המשיח
סורק את קטגוריה 1 בפורום (https://f2.freeivr.co.il/category/1),
משווה מול topics_config.json, שולח התראת מייל על נושאים חדשים דרך Apps Script,
ממיר לעיצוב Markdown עשיר עם קישורים פנימיים וסעיפים מעוגנים, ומעדכן את קובצי התיעוד ב-knowledge/.
"""

import os
import sys
import json
import time
import re
import html
import argparse
import urllib.request
import urllib.parse
from pathlib import Path

# ייבוא מודולי ניקוי וסיווג פוסטים
from cleaner import clean_html_content, extract_post_heading, make_slug
from post_classifier import classify_post

if hasattr(sys.stdout, 'reconfigure'):
    sys.stdout.reconfigure(encoding='utf-8')


BASE_FORUM_URL = "https://f2.freeivr.co.il"
USER_AGENT = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"

def fetch_json(url: str, timeout: int = 30, max_retries: int = 3) -> dict:
    """ביצוע קריאת GET עם 3 ניסיונות חוזרים במקרי כשל, כולל השהייה מדורגת"""
    headers = {"User-Agent": USER_AGENT}
    for attempt in range(max_retries):
        try:
            req = urllib.request.Request(url, headers=headers)
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                data = resp.read().decode("utf-8")
                return json.loads(data)
        except urllib.error.HTTPError as e:
            print(f"[!] שגיאת HTTP {e.code} בפנייה ל-{url} (ניסיון {attempt + 1}/{max_retries})")
            if attempt == max_retries - 1:
                raise
            time.sleep(2 * (attempt + 1))
        except Exception as e:
            print(f"[!] כשל ברשת בפנייה ל-{url}: {e} (ניסיון {attempt + 1}/{max_retries})")
            if attempt == max_retries - 1:
                raise
            time.sleep(2 * (attempt + 1))
    return {}

def post_json(url: str, payload: dict, timeout: int = 30) -> dict:
    """ביצוע קריאת POST והחזרת JSON"""
    headers = {
        "User-Agent": USER_AGENT,
        "Content-Type": "application/json; charset=utf-8"
    }
    data_bytes = json.dumps(payload, ensure_ascii=False).encode("utf-8")
    req = urllib.request.Request(url, data=data_bytes, headers=headers, method="POST")
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        res_text = resp.read().decode("utf-8")
        try:
            return json.loads(res_text)
        except Exception:
            return {"response": res_text}

def get_all_category_topics(category_id: int) -> list:
    """משיכת כל הנושאים בקטגוריה מסוימת כולל כל עמודי הפגינציה"""
    print(f"[*] מושך נושאים מקטגוריה {category_id}...")
    topics = []
    page = 1
    while True:
        url = f"{BASE_FORUM_URL}/api/category/{category_id}?page={page}"
        data = fetch_json(url)
        page_topics = data.get("topics", [])
        if not page_topics:
            break
        topics.extend(page_topics)
        pagination = data.get("pagination", {})
        page_count = pagination.get("pageCount", 1)
        print(f"    עמוד {page}/{page_count}: נמשכו {len(page_topics)} נושאים")
        if page >= page_count:
            break
        page += 1
        time.sleep(0.3)
    print(f"[+] סה\"כ נמשכו {len(topics)} נושאים מהפורום.")
    return topics

def get_topic_all_posts(tid: int) -> list:
    """משיכת כל הפוסטים בנושא מסוים כולל דפדוף"""
    posts = []
    page = 1
    while True:
        url = f"{BASE_FORUM_URL}/api/topic/{tid}?page={page}"
        data = fetch_json(url)
        page_posts = data.get("posts", [])
        if not page_posts:
            break
        posts.extend(page_posts)
        pagination = data.get("pagination", {})
        page_count = pagination.get("pageCount", 1)
        if page >= page_count:
            break
        page += 1
        time.sleep(0.2)
    return posts

class LinkResolver:
    """
    מפענח וממיר קישורי פורום NodeBB לקישורים יחסיים בין קובצי התיעוד וסעיפיהם.
    """
    def __init__(self, config_by_tid: dict, cache_file: Path = None):
        self.config_by_tid = config_by_tid
        self.cache_file = cache_file
        self.redirect_cache = {}
        if self.cache_file and self.cache_file.exists():
            try:
                with open(self.cache_file, "r", encoding="utf-8") as f:
                    self.redirect_cache = json.load(f)
            except Exception:
                pass
        self.post_meta_by_pid = {}
        self.post_meta_by_tid_and_idx = {}
        self.topic_meta_by_tid = {}

    def save_cache(self):
        if self.cache_file:
            try:
                with open(self.cache_file, "w", encoding="utf-8") as f:
                    json.dump(self.redirect_cache, f, ensure_ascii=False, indent=2)
            except Exception:
                pass

    def index_topic(self, tid: int, title: str, target_file: str):
        slug = make_slug(title)
        self.topic_meta_by_tid[tid] = {
            "title": title,
            "target_file": target_file,
            "slug": slug
        }

    def index_post(self, tid: int, pid: int, index: int, heading: str, target_file: str):
        slug = make_slug(heading)
        meta = {
            "tid": tid,
            "pid": pid,
            "index": index,
            "heading": heading,
            "slug": slug,
            "target_file": target_file
        }
        if pid:
            self.post_meta_by_pid[pid] = meta
        self.post_meta_by_tid_and_idx[(tid, index)] = meta

    def get_post_redirect(self, pid: int):
        pid_str = str(pid)
        if pid_str in self.redirect_cache:
            return self.redirect_cache[pid_str]
        try:
            req = urllib.request.Request(f"{BASE_FORUM_URL}/api/post/{pid}", headers={"User-Agent": USER_AGENT})
            with urllib.request.urlopen(req, timeout=5) as resp:
                data = resp.read().decode("utf-8").strip('"')
                m = re.match(r'/topic/(\d+)(?:/[^/]+)?(?:/(\d+))?', data)
                if m:
                    target_tid = int(m.group(1))
                    post_idx = int(m.group(2)) if m.group(2) else 1
                    res = {"tid": target_tid, "index": post_idx}
                    self.redirect_cache[pid_str] = res
                    return res
        except Exception:
            pass
        self.redirect_cache[pid_str] = None
        return None

    def resolve(self, href: str, text: str, current_file: str) -> str:
        """פענוח קישור והמרתו לקישור יחסי בקובץ היעד או השארתו כקישור רשת"""
        if not href:
            return text

        clean_href = re.sub(r'\?_=\d+', '', href).strip()

        target_tid = None
        target_pid = None
        post_idx = None

        # 1. בדיקת /post/PID
        m_post = re.search(r'(?:https?://f2\.freeivr\.co\.il)?/post/(\d+)', clean_href)
        if m_post:
            target_pid = int(m_post.group(1))
            if target_pid in self.post_meta_by_pid:
                meta = self.post_meta_by_pid[target_pid]
                target_tid = meta["tid"]
                post_idx = meta["index"]
            else:
                redir = self.get_post_redirect(target_pid)
                if redir:
                    target_tid = redir["tid"]
                    post_idx = redir["index"]

        # 2. בדיקת /topic/TID
        if not target_tid:
            m_topic = re.search(r'(?:https?://f2\.freeivr\.co\.il)?/topic/(\d+)(?:/[^/?#]+)?(?:/(\d+))?', clean_href)
            if m_topic:
                target_tid = int(m_topic.group(1))
                if m_topic.group(2):
                    post_idx = int(m_topic.group(2))

        # בדיקה האם הנושא שייך לקטגוריה 1 ומנוהל בתיעוד
        if target_tid and target_tid in self.config_by_tid:
            cfg = self.config_by_tid[target_tid]
            if cfg.get("action") != "ignore":
                raw_tf = cfg.get("target_file", "").strip()
                clean_tf = re.sub(r'[<>:"/\\|?*]', '', raw_tf).strip()
                if clean_tf and not clean_tf.endswith('.txt'):
                    clean_tf += '.txt'

                heading = ""
                slug = ""
                if target_pid and target_pid in self.post_meta_by_pid:
                    heading = self.post_meta_by_pid[target_pid]["heading"]
                    slug = self.post_meta_by_pid[target_pid]["slug"]
                elif (target_tid, post_idx) in self.post_meta_by_tid_and_idx:
                    heading = self.post_meta_by_tid_and_idx[(target_tid, post_idx)]["heading"]
                    slug = self.post_meta_by_tid_and_idx[(target_tid, post_idx)]["slug"]
                elif target_tid in self.topic_meta_by_tid:
                    heading = self.topic_meta_by_tid[target_tid]["title"]
                    slug = self.topic_meta_by_tid[target_tid]["slug"]
                else:
                    heading = cfg.get("title", "")
                    slug = make_slug(heading)

                # קביעת נתיב הקישור (האם באותו קובץ או בקובץ אחר)
                if clean_tf == current_file:
                    dest = f"#{slug}" if slug else ""
                else:
                    dest = f"{clean_tf}#{slug}" if slug else clean_tf

                # העשרת כותרת הקישור במידה והיא סתמית (כמו "כאן", "הסבר")
                display_text = text.strip()
                if not display_text:
                    display_text = heading or clean_tf
                elif display_text in ["כאן", "לחץ כאן", "הסבר", "קישור", "שרשור הבא", "שרשור ישן"]:
                    if heading:
                        display_text = f"{display_text} ({heading})"

                return f"[{display_text}]({dest})"

        # אם היעד מחוץ לקטגוריה 1, השארת קישור רשת רגיל
        return f"[{text}]({href})"

def send_gas_alert(gas_url: str, admin_secret: str, unclassified_topics: list):
    """שליחת התראת מייל דרך Google Apps Script על נושאים חדשים שלא סווגו"""
    print(f"[!] שולח התראת מייל על {len(unclassified_topics)} נושאים חדשים דרך Apps Script...")
    payload = {
        "action": "notifyNewTopics",
        "secret": admin_secret,
        "topics": unclassified_topics
    }
    try:
        res = post_json(gas_url, payload)
        print(f"[+] תגובת Apps Script: {res}")
    except Exception as e:
        print(f"[-] שגיאה בשליחת התראה ל-Apps Script: {e}")

def main():
    parser = argparse.ArgumentParser(description="סנכרון תיעוד ימות המשיח")
    parser.add_argument("--config", default=None, help="נתיב לקובץ topics_config.json")
    parser.add_argument("--output-dir", default=None, help="נתיב לתיקיית knowledge/")
    parser.add_argument("--category-id", type=int, default=1, help="מזהה קטגוריית הפורום (ברירת מחדל: 1)")
    parser.add_argument("--dry-run", action="store_true", help="הרצת בדיקה ללא שינוי קבצים וללא שליחת התראות")
    parser.add_argument("--force", action="store_true", help="עדכון קבצים גם אם לא זוהה שינוי")
    parser.add_argument("--limit", type=int, default=None, help="הגבלת מספר הנושאים לסנכרון (לבדיקות)")
    parser.add_argument("--gas-url", default=os.environ.get("GAS_WEBAPP_URL", ""), help="כתובת ה-Web App של Google Apps Script")
    parser.add_argument("--admin-secret", default=os.environ.get("ADMIN_SECRET", "yemot_admin_secret"), help="סוד מנהל להתראות Apps Script")
    args = parser.parse_args()

    script_dir = Path(__file__).resolve().parent
    repo_root = script_dir.parent

    config_path = Path(args.config) if args.config else script_dir / "topics_config.json"
    output_dir = Path(args.output_dir) if args.output_dir else repo_root / "knowledge"

    if not config_path.exists():
        print(f"[-] שגיאה: קובץ ההגדרות לא נמצא: {config_path}")
        sys.exit(1)

    output_dir.mkdir(parents=True, exist_ok=True)

    with open(config_path, "r", encoding="utf-8") as f:
        config_list = json.load(f)

    config_by_tid = {entry["tid"]: entry for entry in config_list}

    # 1. משיכת כל הנושאים מהפורום
    forum_topics = get_all_category_topics(args.category_id)

    # 2. זיהוי נושאים לא מסווגים
    unclassified_topics = []
    for t in forum_topics:
        tid = t.get("tid")
        title = html.unescape(t.get("title", "")).strip()
        if tid not in config_by_tid:
            unclassified_topics.append({
                "tid": tid,
                "title": title,
                "url": f"{BASE_FORUM_URL}/topic/{tid}",
                "postcount": t.get("postcount", 0)
            })

    if unclassified_topics:
        print("\n" + "!" * 60)
        print(f"[!] נמצאו {len(unclassified_topics)} נושאים חדשים שלא סווגו בהגדרות הוורקר:")
        for ut in unclassified_topics:
            print(f"    - [{ut['tid']}] {ut['title']} ({ut['url']})")
        print("!" * 60 + "\n")

        if not args.dry_run and args.gas_url:
            send_gas_alert(args.gas_url, args.admin_secret, unclassified_topics)
    else:
        print("[+] כל הנושאים בפורום מסווגים ומטופלים בהגדרות הוורקר.")

    # 3. קיבוץ נושאים לפי קובץ יעד
    file_to_topics = {}
    ignored_count = 0

    topics_to_process = forum_topics if args.limit is None else forum_topics[:args.limit]

    for t in topics_to_process:
        tid = t.get("tid")
        if tid not in config_by_tid:
            continue
        cfg = config_by_tid[tid]
        action = cfg.get("action", "ignore")
        if action == "ignore":
            ignored_count += 1
            continue

        target_file = cfg.get("target_file", "").strip()
        if not target_file:
            continue

        if target_file not in file_to_topics:
            file_to_topics[target_file] = []

        file_to_topics[target_file].append({
            "tid": tid,
            "cfg": cfg,
            "meta": t
        })

    # 4. משיכת כל הפוסטים ואינדוקס ב-LinkResolver
    print(f"\n[*] מושך פוסטים ומאנדקס קישורים וסעיפים עבור {len(file_to_topics)} קובצי תיעוד...")
    cache_file = script_dir / "redirect_cache.json"
    resolver = LinkResolver(config_by_tid, cache_file=cache_file)
    topic_posts_cache = {}

    for raw_filename, topic_items in file_to_topics.items():
        clean_filename = re.sub(r'[<>:"/\\|?*]', '', raw_filename).strip()
        if not clean_filename.endswith('.txt'):
            clean_filename += '.txt'

        for item in topic_items:
            tid = item["tid"]
            title = html.unescape(item["meta"].get("title", "")).strip()
            resolver.index_topic(tid, title, clean_filename)

            posts = get_topic_all_posts(tid)
            topic_posts_cache[tid] = posts

            for idx, p in enumerate(posts):
                pid = p.get("pid")
                h = extract_post_heading(p.get("content", ""), idx, title)
                resolver.index_post(tid, pid, idx + 1, h, clean_filename)

    resolver.save_cache()
    print("[+] אינדוקס הקישורים והסעיפים הושלם בהצלחה.")

    # 5. עיבוד ויצירת קובצי התיעוד
    print(f"\n[*] מעבד וממיר {len(file_to_topics)} קובצי תיעוד ל-Markdown עשיר...")

    updated_count = 0
    unchanged_count = 0
    created_count = 0

    for raw_filename, topic_items in file_to_topics.items():
        clean_filename = re.sub(r'[<>:"/\\|?*]', '', raw_filename).strip()
        if not clean_filename.endswith('.txt'):
            clean_filename += '.txt'
        file_path = output_dir / clean_filename
        filename = clean_filename

        file_sections = []
        is_multi_topic = len(topic_items) > 1

        for item in topic_items:
            tid = item["tid"]
            cfg = item["cfg"]
            meta = item["meta"]
            topic_title = html.unescape(meta.get("title", "")).strip()
            topic_slug = make_slug(topic_title)
            posts = topic_posts_cache.get(tid, [])

            topic_parts = []
            if is_multi_topic:
                topic_parts.append(f"# {topic_title}\n<a id=\"{topic_slug}\"></a><a id=\"topic-{tid}\"></a>\n")

            valid_post_idx = 0
            for idx, p in enumerate(posts):
                pid = p.get("pid")
                raw_content = p.get("content", "")

                include, reason = classify_post(raw_content, raw_content, pid=pid, post_filter=cfg.get("post_filter", {}))
                if not include:
                    continue

                heading = extract_post_heading(raw_content, idx, topic_title)
                slug = make_slug(heading)
                cleaned = clean_html_content(raw_content, link_resolver=resolver.resolve, current_file=clean_filename)

                if valid_post_idx == 0 and not is_multi_topic:
                    header_line = f"# {topic_title}\n<a id=\"{topic_slug}\"></a><a id=\"topic-{tid}\"></a><a id=\"post-{pid}\"></a>\n\n"
                    topic_parts.append(header_line + cleaned)
                else:
                    section_header = f"\n\n---\n\n<a id=\"{slug}\"></a><a id=\"post-{pid}\"></a>\n## {heading}\n\n"
                    topic_parts.append(section_header + cleaned)

                valid_post_idx += 1

            if topic_parts:
                file_sections.append("\n\n".join(topic_parts).strip())

        if not file_sections:
            continue

        new_content = "\n\n".join(file_sections).strip() + "\n"

        if file_path.exists():
            with open(file_path, "r", encoding="utf-8", errors="ignore") as f:
                old_content = f.read()

            if old_content.strip() == new_content.strip() and not args.force:
                unchanged_count += 1
                continue

            if not args.dry_run:
                with open(file_path, "w", encoding="utf-8") as f:
                    f.write(new_content)
            print(f"    [מעודכן] {filename}")
            updated_count += 1
        else:
            if not args.dry_run:
                with open(file_path, "w", encoding="utf-8") as f:
                    f.write(new_content)
            print(f"    [חדש] {filename}")
            created_count += 1

    resolver.save_cache()

    desktop_knowledge_dir = repo_root / "desktop" / "src-tauri" / "knowledge"
    if desktop_knowledge_dir.exists() and not args.dry_run:
        import shutil
        print("[*] מסנכרן קבצים מעודכנים לתיקיית הדסקטופ (desktop/src-tauri/knowledge)...")
        for f in output_dir.glob("*.txt"):
            shutil.copy2(f, desktop_knowledge_dir / f.name)


    print("\n" + "=" * 50)
    print("סיכום סנכרון תיעוד:")
    print(f"  נושאים שנסרקו: {len(topics_to_process)}")
    print(f"  נושאים שנדחו (התעלמות): {ignored_count}")
    print(f"  נושאים חדשים ללא סיווג: {len(unclassified_topics)}")
    print(f"  קבצים שנוצרו: {created_count}")
    print(f"  קבצים שעודכנו: {updated_count}")
    print(f"  קבצים ללא שינוי: {unchanged_count}")
    print("=" * 50)

if __name__ == "__main__":
    main()

