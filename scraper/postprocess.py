"""
postprocess.py - הרצת ניקוי-העל (postprocess_knowledge_text) על כל קובצי הידע.

אינו דורש רשת כלל. מפעיל את אותו הניקוי שרץ אוטומטית בזמן סנכרון (sync_docs.py),
ולאחר מכן מסנכרן את התוצאה לתיקיית הדסקטופ.

    python scraper/postprocess.py            # ניקוי + העתקה לדסקטופ
    python scraper/postprocess.py --dry-run  # דיווח בלבד, ללא כתיבה
    python scraper/postprocess.py --check     # קוד יציאה 1 אם יש קובץ שאינו נקי
"""

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from cleaner import postprocess_knowledge_text  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parent.parent
KNOWLEDGE_DIR = REPO_ROOT / "knowledge"

CHARS_PER_TOKEN = 2.44


def _token_counter():
    """מונה טוקנים לפי tiktoken (o200k_base) אם מותקן, אחרת אומדן chars/2.44."""
    try:
        import tiktoken
        enc = tiktoken.get_encoding("o200k_base")
        return lambda s: len(enc.encode(s, disallowed_special=())), "tiktoken/o200k_base"
    except Exception:
        return lambda s: int(len(s) / CHARS_PER_TOKEN), f"estimate chars/{CHARS_PER_TOKEN}"


def main():
    ap = argparse.ArgumentParser(description="ניקוי-על לקובצי knowledge (ללא רשת)")
    ap.add_argument("--dry-run", action="store_true", help="ללא כתיבה לדיסק")
    ap.add_argument("--check", action="store_true", help="יציאה בקוד 1 אם קובץ אינו נקי")
    args = ap.parse_args()

    count_tokens, token_mode = _token_counter()

    files = sorted(KNOWLEDGE_DIR.glob("*.txt"))
    if not files:
        print(f"[!] לא נמצאו קבצים ב-{KNOWLEDGE_DIR}")
        return 1

    tot = {"bytes": 0, "chars": 0, "tokens": 0}
    tot_new = {"bytes": 0, "chars": 0, "tokens": 0}
    changed = []

    for path in files:
        original = path.read_text(encoding="utf-8")
        cleaned = postprocess_knowledge_text(original, path.name)

        tot["bytes"] += len(original.encode("utf-8"))
        tot["chars"] += len(original)
        tot["tokens"] += count_tokens(original)
        tot_new["bytes"] += len(cleaned.encode("utf-8"))
        tot_new["chars"] += len(cleaned)
        tot_new["tokens"] += count_tokens(cleaned)

        if cleaned != original:
            changed.append(path.name)
            if not args.dry_run and not args.check:
                path.write_text(cleaned, encoding="utf-8")

    print(f"[*] מדידת טוקנים: {token_mode}")
    print(f"[*] קבצים: {len(files)} | שונו: {len(changed)}")
    print(f"{'מדד':<10}{'לפני':>14}{'אחרי':>14}{'חיסכון':>14}{'%':>9}")
    for key in ("bytes", "chars", "tokens"):
        before, after = tot[key], tot_new[key]
        diff = before - after
        pct = (diff / before * 100) if before else 0.0
        print(f"{key:<10}{before:>14,}{after:>14,}{diff:>14,}{pct:>8.2f}%")

    if args.check:
        if changed:
            print("[!] הקבצים הבאים אינם נקיים:")
            for name in changed:
                print(f"    - {name}")
            return 1
        print("[OK] כל הקבצים נקיים.")
        return 0

    if args.dry_run:
        print("[dry-run] לא נכתב דבר.")
        return 0

    # אין עותק כפול: אפליקציית הדסקטופ מטמיעה ישירות את knowledge/ שבשורש.
    return 0


if __name__ == "__main__":
    sys.exit(main())
