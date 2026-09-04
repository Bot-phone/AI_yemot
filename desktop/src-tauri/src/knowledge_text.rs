//! כללי הטקסט המשותפים למאגר הידע — נרמול, טוקניזציה, אותיות שימוש,
//! חלוקה לכותרות/נתחים וחילוץ מפתחות מבלוקי ```ini.
//!
//! זהו מקור האמת היחיד: הקובץ נכלל פעמיים —
//!   * ב-`build.rs` (בונה האינדקס) דרך `#[path = "src/knowledge_text.rs"]`
//!   * ב-`src/knowledge.rs` (צד השאילתה) דרך `#[path = "knowledge_text.rs"]`
//!
//! לכן אסור שיהיו כאן תלויות מעבר ל-`std`, ואסור להסתמך על פריטים מה-crate.
//! חלק מהפריטים נחוצים רק לצד אחד, ולכן מסומנים `#[allow(dead_code)]` פרטנית.

// ---------------------------------------------------------------------------
// קבועים
// ---------------------------------------------------------------------------

/// גודל נתח מרבי (בהערכת טוקנים) לפני פיצול קשיח.
#[allow(dead_code)] // בשימוש בצד הבנייה בלבד
pub const MAX_CHUNK_TOKENS: usize = 1400;

/// יחס תווים-לטוקן משוער לעברית + לטינית מעורבות.
pub const TOK_DIV: f64 = 2.44;

/// הערכת מספר טוקנים: תווים חלקי 2.44, מעוגל כלפי מעלה.
pub fn estimate_tokens(s: &str) -> usize {
    let n = s.chars().count() as f64;
    (n / TOK_DIV).ceil() as usize
}

// ---------------------------------------------------------------------------
// נרמול וטוקניזציה
// ---------------------------------------------------------------------------

/// הסרת ניקוד/טעמים, מיפוי אותיות סופיות, המרת ASCII לאותיות קטנות.
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if ('\u{0591}'..='\u{05C7}').contains(&c) {
            continue;
        }
        let c = match c {
            '\u{05DA}' => '\u{05DB}', // ך -> כ
            '\u{05DD}' => '\u{05DE}', // ם -> מ
            '\u{05DF}' => '\u{05E0}', // ן -> נ
            '\u{05E3}' => '\u{05E4}', // ף -> פ
            '\u{05E5}' => '\u{05E6}', // ץ -> צ
            _ => c,
        };
        if c.is_ascii_uppercase() {
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

pub fn is_hebrew_letter(c: char) -> bool {
    ('\u{05D0}'..='\u{05EA}').contains(&c)
}

/// טוקניזציה של טקסט מנורמל.
pub fn tokenize(norm: &str) -> Vec<String> {
    let chars: Vec<char> = norm.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if is_hebrew_letter(c) {
            let start = i;
            while i < chars.len() && is_hebrew_letter(chars[i]) {
                i += 1;
            }
            out.push(chars[start..i].iter().collect());
        } else if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(chars[start..i].iter().collect());
        } else if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            out.push(chars[start..i].iter().collect());
        } else {
            i += 1;
        }
    }
    out
}

/// תחיליות (אותיות שימוש) שמורחבות בזמן שאילתה בלבד.
#[allow(dead_code)] // בשימוש בצד השאילתה בלבד
pub const CLITICS: &[&str] = &[
    "ומש", "וכש", "ושה", "מה", "שה", "כש", "לה", "בה", "וה", "ול", "וב", "ומ", "וכ", "ו", "ה", "ב",
    "ל", "מ", "ש", "כ",
];

/// מרחיב מונח שאילתה לתחיליות אפשריות (רק אם נשארות לפחות 2 אותיות).
#[allow(dead_code)] // בשימוש בצד השאילתה בלבד
pub fn expand_clitics(term: &str) -> Vec<String> {
    let mut out = vec![term.to_string()];
    let chars: Vec<char> = term.chars().collect();
    if !chars.first().copied().map(is_hebrew_letter).unwrap_or(false) {
        return out;
    }
    for p in CLITICS {
        let plen = p.chars().count();
        if chars.len() < plen + 2 {
            continue;
        }
        let head: String = chars[..plen].iter().collect();
        if head == *p {
            let rest: String = chars[plen..].iter().collect();
            if !out.contains(&rest) {
                out.push(rest);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// כותרות וחלוקה לנתחים
// ---------------------------------------------------------------------------

/// כותרת `##` / `###` בשורה נתונה, מנוקה מהדגשות `**`.
#[allow(dead_code)] // בשימוש בצד הבנייה בלבד
pub fn heading_of(line: &str) -> Option<String> {
    let t = line.trim_end_matches(['\r', '\n']);
    let hashes = t.chars().take_while(|c| *c == '#').count();
    if !(hashes == 2 || hashes == 3) {
        return None;
    }
    let rest = &t[hashes..];
    if !rest.starts_with(' ') {
        return None;
    }
    let mut h = rest.trim().to_string();
    while h.starts_with('*') && h.ends_with('*') && h.chars().count() > 2 {
        h = h.trim_matches('*').trim().to_string();
    }
    Some(h)
}

/// חלוקת קובץ ל-(start, end, heading) לפי כותרות `##` / `###`.
#[allow(dead_code)] // בשימוש בצד הבנייה (ובבדיקת השפיות בצד השאילתה)
pub fn split_sections(text: &str) -> Vec<(usize, usize, String)> {
    let mut marks: Vec<(usize, String)> = Vec::new();
    let mut off = 0usize;
    for line in text.split_inclusive('\n') {
        if let Some(h) = heading_of(line) {
            marks.push((off, h));
        }
        off += line.len();
    }
    let mut out = Vec::new();
    let first = marks.first().map(|m| m.0).unwrap_or(text.len());
    if first > 0 {
        out.push((0usize, first, String::new()));
    }
    for (i, (s, h)) in marks.iter().enumerate() {
        let e = marks.get(i + 1).map(|m| m.0).unwrap_or(text.len());
        out.push((*s, e, h.clone()));
    }
    if out.is_empty() {
        out.push((0, text.len(), String::new()));
    }
    // איחוד מקטע שגופו ריק (כותרת H2 שמיד אחריה כותרת H3 זהה) עם הבא אחריו,
    // כדי לא ליצור נתחים זעירים חסרי תוכן.
    let mut merged: Vec<(usize, usize, String)> = Vec::with_capacity(out.len());
    for sec in out.into_iter().rev() {
        let body_after_heading = match text[sec.0..sec.1].find('\n') {
            Some(nl) => &text[sec.0 + nl..sec.1],
            None => "",
        };
        if body_after_heading.trim().is_empty() && !merged.is_empty() {
            let last = merged.len() - 1;
            merged[last].0 = sec.0;
            merged[last].2 = sec.2;
        } else {
            merged.push(sec);
        }
    }
    merged.reverse();
    merged
}

/// פיצול קשיח של מקטע גדול מדי על גבולות שורה.
#[allow(dead_code)] // בשימוש בצד הבנייה (ובבדיקת השפיות בצד השאילתה)
pub fn hard_split(text: &str, start: usize, end: usize) -> Vec<(usize, usize)> {
    let slice = &text[start..end];
    if estimate_tokens(slice) <= MAX_CHUNK_TOKENS {
        return vec![(start, end)];
    }
    let mut parts = Vec::new();
    let mut cur_start = start;
    let mut cur_chars = 0usize;
    let mut off = start;
    let budget_chars = (MAX_CHUNK_TOKENS as f64 * TOK_DIV) as usize;
    for line in slice.split_inclusive('\n') {
        let lc = line.chars().count();
        if cur_chars > 0 && cur_chars + lc > budget_chars {
            parts.push((cur_start, off));
            cur_start = off;
            cur_chars = 0;
        }
        cur_chars += lc;
        off += line.len();
    }
    if off > cur_start {
        parts.push((cur_start, off));
    }
    if parts.is_empty() {
        parts.push((start, end));
    }
    parts
}

/// מספר הנתחים שהבנייה תייצר עבור טקסט קובץ יחיד.
#[allow(dead_code)] // בשימוש בבדיקת השפיות בצד השאילתה
pub fn chunk_count(text: &str) -> usize {
    split_sections(text)
        .into_iter()
        .map(|(s, e, _)| hard_split(text, s, e).len())
        .sum()
}

// ---------------------------------------------------------------------------
// חילוץ מפתחות
// ---------------------------------------------------------------------------

/// `type=menu`, `type = menu`, `- [type=menu]` -> Some("menu")
#[allow(dead_code)] // בשימוש בצד הבנייה בלבד
pub fn extract_kv(line: &str, key: &str) -> Option<String> {
    let l = line.trim().trim_start_matches(['-', '*', ' ', '[']).trim();
    let rest = l.strip_prefix(key)?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let v: String = rest
        .chars()
        .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')
        .collect();
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

/// `key=value` בתוך גדר ```ini -> Some("key")
#[allow(dead_code)] // בשימוש בצד הבנייה בלבד
pub fn extract_key(line: &str) -> Option<String> {
    let l = line.trim();
    if l.is_empty() || l.starts_with('#') || l.starts_with(';') || l.starts_with('[') {
        return None;
    }
    let eq = l.find('=')?;
    let k = l[..eq].trim();
    if k.is_empty() || k.len() > 64 {
        return None;
    }
    // מפתחות מספריים (כמו `1=` בתפריט) אינם שמות פרמטרים מתועדים
    if k.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if !k
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
    {
        return None;
    }
    Some(k.to_ascii_lowercase())
}

/// מצב מעקב אחרי גדרות ```ini בתוך גוף נתח.
/// מחזיר `true` אם השורה היא גדר (ולכן אינה שורת תוכן).
#[allow(dead_code)] // בשימוש בצד הבנייה בלבד
pub fn track_ini_fence(line_trimmed: &str, in_ini: &mut bool) -> bool {
    if !line_trimmed.starts_with("```") {
        return false;
    }
    if *in_ini {
        *in_ini = false;
    } else {
        let lang = line_trimmed
            .trim_start_matches('`')
            .trim()
            .to_ascii_lowercase();
        *in_ini = lang == "ini" || lang.is_empty();
    }
    true
}
