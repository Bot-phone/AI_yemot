#!/usr/bin/env python3
"""
sync_docs.py - וורקר לסנכרון, סיווג, ניקוי ושמירת תיעוד ימות המשיח
סורק את קטגוריה 1 בפורום (https://f2.freeivr.co.il/category/1),
משווה מול topics_config.json, שולח התראת מייל על נושאים חדשים דרך Apps Script,
ומעדכן את קובצי התיעוד בתיקיית knowledge/.
"""

import os
import sys
import json
import time
import html
import argparse
import urllib.request
import urllib.parse
from pathlib import Path

# ייבוא מודול הניקוי
from cleaner import clean_html_content

if hasattr(sys.stdout, 'reconfigure'):
    sys.stdout.reconfigure(encoding='utf-8')


BASE_FORUM_URL = "https://f2.freeivr.co.il"
USER_AGENT = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"

def fetch_json(url: str, timeout: int = 30) -> dict:
    """ביצוע קריאת GET והחזרת JSON עם מנגנון ניסיונות חוזרים"""
    headers = {"User-Agent": USER_AGENT}
    for attempt in range(3):
        try:
            req = urllib.request.Request(url, headers=headers)
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                data = resp.read().decode("utf-8")
                return json.loads(data)
        except Exception as e:
            if attempt == 2:
                raise
            time.sleep(1.5 * (attempt + 1))
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

def filter_and_clean_posts(posts: list, post_filter: dict, topic_author: str) -> list:
    """סינון פוסטים וניקוי תוכן לפי כללי הסינון של הנושא"""
    mode = post_filter.get("mode", "author_only")
    allowed_authors = set(post_filter.get("authors", []))
    if topic_author:
        allowed_authors.add(topic_author)
    
    exclude_pids = set(post_filter.get("exclude_pids", []))
    include_pids = set(post_filter.get("include_pids", []))
    min_length = post_filter.get("min_length", 20)

    cleaned_texts = []
    for p in posts:
        pid = p.get("pid")
        if pid in exclude_pids:
            continue

        raw_content = p.get("content", "")
        author = p.get("user", {}).get("username", "")

        # בדיקת אישור פוסט
        accept = False
        if pid in include_pids:
            accept = True
        elif mode == "all":
            accept = True
        elif mode == "author_only":
            if author in allowed_authors:
                accept = True

        if not accept:
            continue

        cleaned = clean_html_content(raw_content)
        if len(cleaned) >= min_length:
            cleaned_texts.append(cleaned)

    return cleaned_texts

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
    # file_to_topics: { "filename.txt": [ {tid, config, topic_meta}, ... ] }
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

    print(f"\n[*] מעבד {len(file_to_topics)} קובצי תיעוד (התעלמות מ-{ignored_count} נושאים)...")

    updated_count = 0
    unchanged_count = 0
    created_count = 0

    for filename, topic_items in file_to_topics.items():
        file_path = output_dir / filename
        
        # איסוף התוכן מכל הנושאים המיועדים לקובץ זה
        file_sections = []
        for item in topic_items:
            tid = item["tid"]
            cfg = item["cfg"]
            meta = item["meta"]
            topic_author = meta.get("user", {}).get("username", "")

            posts = get_topic_all_posts(tid)
            cleaned_posts = filter_and_clean_posts(posts, cfg.get("post_filter", {}), topic_author)
            if cleaned_posts:
                section_text = "\n\n".join(cleaned_posts)
                file_sections.append(section_text)

        if not file_sections:
            continue

        new_content = "\n\n".join(file_sections).strip() + "\n"

        # בדיקה האם הקובץ קיים והאם התוכן השתנה
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
