//! מאגר הידע המוטמע + מנוע חיפוש לקסיקלי (BM25) שנבנה בזמן קומפילציה.
//!
//! האינדקס נבנה ב-`build.rs` ונטען בעצלתיים (`OnceLock`) מתוך
//! `$OUT_DIR/knowledge_index.bin` (פורמט varint קומפקטי, ללא תלויות נוספות).
//!
//! כללי הנרמול/הטוקניזציה/החלוקה משותפים עם `build.rs` דרך המודול
//! `knowledge_text` — מקור אמת יחיד, כך שהאינדקס והשאילתות לא יכולים להיפרד.

/// כללי הטקסט המשותפים; אותו קובץ נכלל גם ב-`build.rs`.
#[path = "knowledge_text.rs"]
pub mod knowledge_text;

use include_dir::{include_dir, Dir};
use knowledge_text::{expand_clitics, normalize, tokenize};
pub use knowledge_text::estimate_tokens;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

// הטמעת כל תיקיית התיעוד והידע בתוך קובץ ה-Binary בזמן הקומפילציה
static KNOWLEDGE_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/knowledge");

/// האינדקס הבינארי שנבנה ב-build.rs
static INDEX_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/knowledge_index.bin"));

// ---------------------------------------------------------------------------
// פקודות Tauri קיימות (ה-UI מסתמך עליהן — לא לשנות חתימות)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct KnowledgeFileItem {
    pub name: String,
    pub size: usize,
}

#[tauri::command]
pub fn get_knowledge_files() -> Vec<KnowledgeFileItem> {
    let mut files = Vec::new();
    for file in KNOWLEDGE_DIR.files() {
        if let Some(name) = file.path().to_str() {
            files.push(KnowledgeFileItem {
                name: name.to_string(),
                size: file.contents().len(),
            });
        }
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));
    files
}

#[tauri::command]
pub fn get_knowledge_file_content(file_name: &str) -> Result<String, String> {
    if let Some(file) = KNOWLEDGE_DIR.get_file(file_name) {
        String::from_utf8(file.contents().to_vec()).map_err(|e| format!("שגיאת פענוח תוכן: {}", e))
    } else {
        Err(format!("קובץ הידע '{}' לא נמצא", file_name))
    }
}

#[tauri::command]
pub fn search_knowledge_files(query: &str) -> Vec<KnowledgeFileItem> {
    let q = query.trim().to_lowercase();
    let all = get_knowledge_files();
    if q.is_empty() {
        return all;
    }
    all.into_iter()
        .filter(|f| f.name.to_lowercase().contains(&q))
        .collect()
}

// ---------------------------------------------------------------------------
// פרמטרים של הדירוג (הנרמול/הטוקניזציה נמצאים ב-knowledge_text.rs)
// ---------------------------------------------------------------------------

const BM25_K1: f64 = 2.5;
const BM25_B: f64 = 0.75;
const HEADING_BOOST: f64 = 3.0;
/// שם הקובץ הוא נושא המסמך במאגר הזה — התאמה אליו היא אות טופיקלי חזק.
const TITLE_BOOST: f64 = 3.0;
/// אורך מסמך מינימלי אפקטיבי — מונע העדפה מנופחת של נתחים זעירים.
const MIN_DOC_LEN: f64 = 40.0;
/// מונח שמופיע ביותר מ-20% מהנתחים נחשב מילת-רקע ומסונן מהשאילתה.
const STOPWORD_DF_RATIO: f64 = 0.20;

// ---------------------------------------------------------------------------
// טעינת האינדקס
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Posting {
    chunk: u32,
    tf: u16,
    in_heading: bool,
}

#[derive(Debug)]
struct Chunk {
    file: u32,
    start: u32,
    end: u32,
    heading: String,
    part: u16,
    nparts: u16,
    doc_len: u32,
}

#[derive(Debug)]
struct Index {
    files: Vec<String>,
    /// טוקנים של שם כל קובץ (ללא סיומת), לחישוב חיזוק כותרת בזמן שאילתה
    file_tokens: Vec<HashSet<String>>,
    chunks: Vec<Chunk>,
    postings: HashMap<String, Vec<Posting>>,
    type_index: HashMap<String, Vec<u32>>,
    param_index: HashMap<String, Vec<u32>>,
    avgdl: f64,
}

static INDEX: OnceLock<Index> = OnceLock::new();

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn uvar(&mut self) -> u64 {
        let mut v = 0u64;
        let mut shift = 0u32;
        loop {
            let byte = self.b[self.i];
            self.i += 1;
            v |= ((byte & 0x7f) as u64) << shift;
            if byte & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        v
    }
    fn byte(&mut self) -> u8 {
        let b = self.b[self.i];
        self.i += 1;
        b
    }
    fn string(&mut self) -> String {
        let n = self.uvar() as usize;
        let s = String::from_utf8_lossy(&self.b[self.i..self.i + n]).into_owned();
        self.i += n;
        s
    }
}

fn index() -> &'static Index {
    INDEX.get_or_init(|| {
        let mut r = Reader {
            b: INDEX_BYTES,
            i: 0,
        };
        assert_eq!(&r.b[0..4], b"KIX1", "index magic mismatch");
        r.i = 4;

        let nf = r.uvar() as usize;
        let mut files = Vec::with_capacity(nf);
        for _ in 0..nf {
            files.push(r.string());
        }

        let nc = r.uvar() as usize;
        let mut chunks = Vec::with_capacity(nc);
        let mut total_len = 0u64;
        for _ in 0..nc {
            let file = r.uvar() as u32;
            let start = r.uvar() as u32;
            let len = r.uvar() as u32;
            let heading = r.string();
            let part = r.uvar() as u16;
            let nparts = r.uvar() as u16;
            let doc_len = r.uvar() as u32;
            total_len += doc_len as u64;
            chunks.push(Chunk {
                file,
                start,
                end: start + len,
                heading,
                part,
                nparts,
                doc_len,
            });
        }

        let nt = r.uvar() as usize;
        let mut postings: HashMap<String, Vec<Posting>> = HashMap::with_capacity(nt);
        for _ in 0..nt {
            let term = r.string();
            let np = r.uvar() as usize;
            let mut v = Vec::with_capacity(np);
            let mut prev = 0u32;
            for _ in 0..np {
                let d = r.uvar() as u32;
                let chunk = prev + d;
                prev = chunk;
                let tf = r.uvar() as u16;
                let in_heading = r.byte() != 0;
                v.push(Posting {
                    chunk,
                    tf,
                    in_heading,
                });
            }
            postings.insert(term, v);
        }

        let read_map = |r: &mut Reader| {
            let n = r.uvar() as usize;
            let mut m: HashMap<String, Vec<u32>> = HashMap::with_capacity(n);
            for _ in 0..n {
                let k = r.string();
                let c = r.uvar() as usize;
                let mut v = Vec::with_capacity(c);
                for _ in 0..c {
                    v.push(r.uvar() as u32);
                }
                m.insert(k, v);
            }
            m
        };
        let type_index = read_map(&mut r);
        let param_index = read_map(&mut r);

        let avgdl = if nc == 0 {
            1.0
        } else {
            (total_len as f64 / nc as f64).max(1.0)
        };

        let file_tokens = files
            .iter()
            .map(|f| {
                tokenize(&normalize(f.strip_suffix(".txt").unwrap_or(f)))
                    .into_iter()
                    .collect()
            })
            .collect();

        Index {
            files,
            file_tokens,
            chunks,
            postings,
            type_index,
            param_index,
            avgdl,
        }
    })
}

/// טקסט הנתח כפי שהוא, מתוך הקובץ המוטמע.
fn chunk_text(idx: &'static Index, ci: u32) -> &'static str {
    let ch = &idx.chunks[ci as usize];
    let name = &idx.files[ch.file as usize];
    match KNOWLEDGE_DIR.get_file(name) {
        Some(f) => {
            let bytes = f.contents();
            let s = ch.start as usize;
            let e = (ch.end as usize).min(bytes.len());
            if s >= e {
                return "";
            }
            std::str::from_utf8(&bytes[s..e]).unwrap_or("")
        }
        None => "",
    }
}

fn file_label(idx: &'static Index, ci: u32) -> &'static str {
    let ch = &idx.chunks[ci as usize];
    let name: &'static str = &idx.files[ch.file as usize];
    name.strip_suffix(".txt").unwrap_or(name)
}

fn heading_label(h: &str) -> &str {
    if h.is_empty() {
        "(פתיח)"
    } else {
        h
    }
}

// ---------------------------------------------------------------------------
// חיפוש
// ---------------------------------------------------------------------------

// עדיין לא נצרך בתוך ה-crate — לולאת הסוכן (agent/) נבנית בנפרד.
#[allow(dead_code)]
pub struct SearchOpts {
    /// ברירת מחדל 5, מקסימום 10
    pub top_k: usize,
    /// סינון לקובץ מסוים (עם או בלי סיומת .txt)
    pub file: Option<String>,
    /// ברירת מחדל 3000, מקסימום 8000
    pub max_tokens: usize,
    /// סמן המשך שהוחזר בקריאה קודמת
    pub cursor: Option<String>,
}

impl Default for SearchOpts {
    fn default() -> Self {
        SearchOpts {
            top_k: 5,
            file: None,
            max_tokens: 3000,
            cursor: None,
        }
    }
}

/// דירוג הנתחים לשאילתה נתונה (דטרמיניסטי).
fn rank(idx: &Index, query: &str, file_filter: Option<&str>) -> Vec<u32> {
    rank_scored(idx, query, file_filter).into_iter().map(|(c, _)| c).collect()
}

fn rank_scored(idx: &Index, query: &str, file_filter: Option<&str>) -> Vec<(u32, f64)> {
    let qtokens = tokenize(&normalize(query));

    let allowed: Option<HashSet<u32>> = file_filter.map(|f| {
        let want = normalize(f.strip_suffix(".txt").unwrap_or(f));
        let mut set = HashSet::new();
        for (fi, name) in idx.files.iter().enumerate() {
            let n = normalize(name.strip_suffix(".txt").unwrap_or(name));
            if n == want || n.contains(&want) {
                set.insert(fi as u32);
            }
        }
        set
    });
    let keep = |ci: u32| -> bool {
        match &allowed {
            None => true,
            Some(s) => s.contains(&idx.chunks[ci as usize].file),
        }
    };

    let n = idx.chunks.len() as f64;
    let df_cutoff = (n * STOPWORD_DF_RATIO) as usize;

    // כל מונח שאילתה + הרחבות אותיות השימוש שלו = "קבוצה" אחת.
    // בתוך קבוצה לוקחים את הניקוד המרבי (ולא סכום) כדי לא לספור פעמיים
    // את אותה מילה בשתי צורות.
    let mut groups: Vec<Vec<&String>> = Vec::new();
    let mut seen_groups: Vec<String> = Vec::new();
    for t in &qtokens {
        if seen_groups.contains(t) {
            continue;
        }
        seen_groups.push(t.clone());
        let variants = expand_clitics(t);
        let g: Vec<&String> = variants
            .iter()
            .filter_map(|v| idx.postings.get_key_value(v.as_str()).map(|(k, _)| k))
            .collect();
        if !g.is_empty() {
            groups.push(g);
        }
    }

    // סינון מונחים נפוצים מדי (מילות קישור, ספרות בודדות וכו').
    // אם לא נשארה אף קבוצה — משאירים הכל כדי לא להחזיר תוצאה ריקה.
    let selective: Vec<Vec<&String>> = groups
        .iter()
        .filter(|g| {
            g.iter()
                .any(|t| idx.postings[t.as_str()].len() <= df_cutoff)
        })
        .cloned()
        .collect();
    let groups = if selective.is_empty() { groups } else { selective };

    let mut scores: HashMap<u32, f64> = HashMap::new();
    for g in &groups {
        let mut best: HashMap<u32, f64> = HashMap::new();
        for t in g {
            let plist = &idx.postings[t.as_str()];
            let df = plist.len() as f64;
            if df as usize > df_cutoff && g.len() > 1 {
                continue; // צורה נפוצה מדי בתוך קבוצה — מדלגים
            }
            let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
            for p in plist {
                if !keep(p.chunk) {
                    continue;
                }
                let dl = (idx.chunks[p.chunk as usize].doc_len as f64).max(MIN_DOC_LEN);
                let tf = p.tf as f64;
                let denom = tf + BM25_K1 * (1.0 - BM25_B + BM25_B * dl / idx.avgdl);
                let mut s = idf * (tf * (BM25_K1 + 1.0)) / denom;
                if p.in_heading {
                    s *= HEADING_BOOST;
                }
                if idx.file_tokens[idx.chunks[p.chunk as usize].file as usize]
                    .contains(t.as_str())
                {
                    s *= TITLE_BOOST;
                }
                let e = best.entry(p.chunk).or_insert(0.0);
                if s > *e {
                    *e = s;
                }
            }
        }
        for (c, s) in best {
            *scores.entry(c).or_insert(0.0) += s;
        }
    }

    let mut ranked: Vec<(u32, f64)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });

    // force-merge: מונחים שהם type= או key= מוכרים מקבלים עדיפות עליונה
    let mut forced: Vec<u32> = Vec::new();
    for t in &qtokens {
        // רק מזהים לטיניים סבירים (לא ספרות בודדות ולא מילים בעברית)
        if t.len() < 3 || !t.is_ascii() || !t.chars().any(|c| c.is_ascii_alphabetic()) {
            continue;
        }
        if let Some(v) = idx.type_index.get(t.as_str()) {
            for c in v {
                if keep(*c) && !forced.contains(c) {
                    forced.push(*c);
                }
            }
        }
        if let Some(v) = idx.param_index.get(t.as_str()) {
            for c in v {
                if keep(*c) && !forced.contains(c) {
                    forced.push(*c);
                }
            }
        }
    }

    let top = ranked.first().map(|r| r.1).unwrap_or(1.0).max(1.0);
    let mut out: Vec<(u32, f64)> = forced.iter().map(|c| (*c, top * 2.0)).collect();
    let seen: HashSet<u32> = forced.iter().copied().collect();
    for (c, sc) in ranked {
        if !seen.contains(&c) {
            out.push((c, sc));
        }
    }
    out
}

fn parse_cursor(c: Option<&str>) -> usize {
    c.and_then(|s| s.strip_prefix('c'))
        .and_then(|s| usize::from_str_radix(s, 16).ok())
        .unwrap_or(0)
}

/// חיפוש במאגר הידע — מחזיר טקסט שטוח המיועד למודל.
// עדיין לא נצרך בתוך ה-crate — לולאת הסוכן (agent/) נבנית בנפרד.
#[allow(dead_code)]
pub fn search_knowledge(query: &str, opts: SearchOpts) -> String {
    let idx = index();
    let top_k = opts.top_k.clamp(1, 10);
    let max_tokens = opts.max_tokens.clamp(200, 8000);
    let start = parse_cursor(opts.cursor.as_deref());

    let ranked = rank(idx, query, opts.file.as_deref());
    let total = ranked.len();
    if total == 0 {
        return format!("לא נמצאו תוצאות עבור: {}", query.trim());
    }

    let mut out = String::new();
    let mut used = 0usize;
    let mut emitted = 0usize;
    let mut i = start;
    let mut oversize_next: Option<String> = None;

    while i < total && emitted < top_k {
        let ci = ranked[i];
        let text = chunk_text(idx, ci);
        let tok = estimate_tokens(text);
        let ch = &idx.chunks[ci as usize];

        if used + tok > max_tokens {
            if emitted == 0 {
                // נתח בודד גדול מדי — מחזירים שורה-שורה עד לתקציב
                let mut body = String::new();
                let mut btok = 0usize;
                let mut consumed_all = true;
                for line in text.split_inclusive('\n') {
                    let lt = estimate_tokens(line);
                    if btok + lt > max_tokens && !body.is_empty() {
                        consumed_all = false;
                        break;
                    }
                    body.push_str(line);
                    btok += lt;
                }
                out.push_str(&format!(
                    "[1] {} › {}   ({} tok)\n{}\n",
                    file_label(idx, ci),
                    heading_label(&ch.heading),
                    btok,
                    body.trim_end()
                ));
                if !consumed_all || ch.part < ch.nparts {
                    oversize_next = Some(format!(
                        "{}#{}~{}",
                        file_label(idx, ci),
                        ch.heading,
                        ch.part + 1
                    ));
                }
                emitted = 1;
                i += 1;
            }
            break;
        }

        out.push_str(&format!(
            "[{}] {} › {}   ({} tok)\n{}\n\n",
            emitted + 1,
            file_label(idx, ci),
            heading_label(&ch.heading),
            tok,
            text.trim_end()
        ));
        used += tok;
        emitted += 1;
        i += 1;
    }

    out.push_str("---\n");
    out.push_str(&format!("נמצאו {} תוצאות, הוחזרו {}.", total, emitted));
    if let Some(next) = oversize_next {
        out.push_str(&format!(" next: \"{}\"", next));
    } else if i < total {
        out.push_str(&format!(" להמשך: cursor=\"c{:x}\"", i));
    }
    out.push('\n');
    out
}

// ---------------------------------------------------------------------------
// שליפת קטע מקובץ
// ---------------------------------------------------------------------------

fn resolve_file(idx: &Index, file: &str) -> Option<u32> {
    let want = normalize(file.strip_suffix(".txt").unwrap_or(file).trim());
    for (fi, name) in idx.files.iter().enumerate() {
        if normalize(name.strip_suffix(".txt").unwrap_or(name)) == want {
            return Some(fi as u32);
        }
    }
    // התאמה חלקית יחידה
    let hits: Vec<u32> = idx
        .files
        .iter()
        .enumerate()
        .filter(|(_, n)| normalize(n).contains(&want))
        .map(|(i, _)| i as u32)
        .collect();
    if hits.len() == 1 {
        return Some(hits[0]);
    }
    None
}

/// שלושת שמות הקבצים הקרובים ביותר לפי חפיפת טוקנים מנורמלים.
fn closest_files(idx: &Index, file: &str) -> Vec<String> {
    let q: HashSet<String> = tokenize(&normalize(file)).into_iter().collect();
    let mut scored: Vec<(usize, usize, &String)> = idx
        .files
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let t: HashSet<String> = tokenize(&normalize(name)).into_iter().collect();
            (q.intersection(&t).count(), i, name)
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored
        .into_iter()
        .take(3)
        .map(|(_, _, n)| n.strip_suffix(".txt").unwrap_or(n).to_string())
        .collect()
}

/// שליפת קטע לפי כותרת. `heading=None` מחזיר פתיח + תוכן עניינים.
// עדיין לא נצרך בתוך ה-crate — לולאת הסוכן (agent/) נבנית בנפרד.
#[allow(dead_code)]
pub fn get_knowledge_section(
    file: &str,
    heading: Option<&str>,
    cursor: Option<&str>,
) -> Result<String, String> {
    let idx = index();
    let Some(fi) = resolve_file(idx, file) else {
        let near = closest_files(idx, file);
        return Err(format!(
            "קובץ הידע '{}' לא נמצא. אולי התכוונת ל: {}",
            file,
            near.join(" | ")
        ));
    };

    let chunks: Vec<u32> = idx
        .chunks
        .iter()
        .enumerate()
        .filter(|(_, c)| c.file == fi)
        .map(|(i, _)| i as u32)
        .collect();
    let fname = idx.files[fi as usize]
        .strip_suffix(".txt")
        .unwrap_or(&idx.files[fi as usize]);

    // סמן המשך בצורה "<file>#<heading>~<part>"
    let mut want_part: u16 = 1;
    let mut heading = heading.map(|h| h.to_string());
    if let Some(c) = cursor {
        if let Some((h, p)) = c.rsplit_once('~') {
            if let Some((_f, hh)) = h.split_once('#') {
                heading = Some(hh.to_string());
            }
            want_part = p.parse().unwrap_or(1);
        }
    }

    let Some(h) = heading else {
        // פתיח + תוכן עניינים
        let mut out = String::new();
        if let Some(&intro) = chunks.first() {
            if idx.chunks[intro as usize].heading.is_empty() {
                out.push_str(chunk_text(idx, intro).trim_end());
                out.push_str("\n\n");
            }
        }
        out.push_str(&format!("--- תוכן עניינים של {} ---\n", fname));
        let mut seen: Vec<&str> = Vec::new();
        for ci in &chunks {
            let hh = &idx.chunks[*ci as usize].heading;
            if hh.is_empty() || seen.contains(&hh.as_str()) {
                continue;
            }
            seen.push(hh.as_str());
            out.push_str(&format!("- {}\n", hh));
        }
        return Ok(out);
    };

    let nh = normalize(&h);
    let matches: Vec<u32> = chunks
        .iter()
        .copied()
        .filter(|ci| {
            let ch = &idx.chunks[*ci as usize];
            let n = normalize(&ch.heading);
            n == nh || (!nh.is_empty() && n.contains(&nh))
        })
        .collect();
    if matches.is_empty() {
        let mut avail: Vec<&str> = Vec::new();
        for ci in &chunks {
            let hh = idx.chunks[*ci as usize].heading.as_str();
            if !hh.is_empty() && !avail.contains(&hh) {
                avail.push(hh);
            }
        }
        return Err(format!(
            "הכותרת '{}' לא נמצאה בקובץ '{}'. כותרות קיימות: {}",
            h,
            fname,
            avail.join(" | ")
        ));
    }

    let budget = 3000usize;
    let mut out = String::new();
    let mut used = 0usize;
    let mut next: Option<String> = None;
    for ci in &matches {
        let ch = &idx.chunks[*ci as usize];
        if ch.part < want_part {
            continue;
        }
        let text = chunk_text(idx, *ci);
        let tok = estimate_tokens(text);
        if used > 0 && used + tok > budget {
            next = Some(format!("{}#{}~{}", fname, ch.heading, ch.part));
            break;
        }
        out.push_str(&format!(
            "[{} › {}]   ({} tok)\n{}\n\n",
            fname,
            heading_label(&ch.heading),
            tok,
            text.trim_end()
        ));
        used += tok;
    }
    if let Some(n) = next {
        out.push_str(&format!("---\nnext: \"{}\"\n", n));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// חיפוש פרמטר
// ---------------------------------------------------------------------------

/// כל המופעים המתועדים של הגדרה, עם הקשר. מוגבל לכ-3000 טוקנים.
// עדיין לא נצרך בתוך ה-crate — לולאת הסוכן (agent/) נבנית בנפרד.
#[allow(dead_code)]
pub fn lookup_param(key: &str) -> String {
    let idx = index();
    let k = normalize(key.trim()).trim_matches('=').to_string();
    let Some(chunks) = idx.param_index.get(&k) else {
        // אולי זה סוג שלוחה ולא פרמטר
        if let Some(v) = idx.type_index.get(&k) {
            let mut out = format!("'{}' הוא סוג שלוחה (type=), לא פרמטר.\n\n", k);
            for ci in v.iter().take(3) {
                let ch = &idx.chunks[*ci as usize];
                out.push_str(&format!(
                    "- {} › {}\n",
                    file_label(idx, *ci),
                    heading_label(&ch.heading)
                ));
            }
            return out;
        }
        return format!("ההגדרה '{}' לא נמצאה במאגר הידע.", k);
    };

    let budget = 3000usize;
    let mut out = format!("הגדרה: {}=\nמופעים מתועדים ({}):\n\n", k, chunks.len());
    let mut used = estimate_tokens(&out);
    let mut shown = 0usize;
    for ci in chunks {
        let ch = &idx.chunks[*ci as usize];
        let text = chunk_text(idx, *ci);
        let lines: Vec<&str> = text.lines().collect();
        let mut snippet = String::new();
        for (li, line) in lines.iter().enumerate() {
            let lt = line.trim();
            let is_hit = lt
                .split('=')
                .next()
                .map(|p| normalize(p.trim()) == k)
                .unwrap_or(false)
                && lt.contains('=');
            if !is_hit {
                continue;
            }
            let from = li.saturating_sub(1);
            let to = (li + 2).min(lines.len());
            for l in &lines[from..to] {
                if !snippet.contains(l) {
                    snippet.push_str(l);
                    snippet.push('\n');
                }
            }
            snippet.push('\n');
        }
        if snippet.trim().is_empty() {
            continue;
        }
        let block = format!(
            "[{} › {}]\n{}\n",
            file_label(idx, *ci),
            heading_label(&ch.heading),
            snippet.trim_end()
        );
        let bt = estimate_tokens(&block);
        if used + bt > budget && shown > 0 {
            out.push_str(&format!(
                "---\n(הוצגו {} מתוך {} מופעים)\n",
                shown,
                chunks.len()
            ));
            return out;
        }
        out.push_str(&block);
        out.push('\n');
        used += bt;
        shown += 1;
    }
    out
}

/// האם המחרוזת היא הגדרה (key=) מוכרת במאגר.
// עדיין לא נצרך בתוך ה-crate — לולאת הסוכן (agent/) נבנית בנפרד.
#[allow(dead_code)]
pub fn is_known_param(key: &str) -> bool {
    let k = normalize(key.trim()).trim_matches('=').to_string();
    index().param_index.contains_key(&k)
}

/// האם המחרוזת היא סוג שלוחה (type=) מוכר.
// עדיין לא נצרך בתוך ה-crate — לולאת הסוכן (agent/) נבנית בנפרד.
#[allow(dead_code)]
pub fn is_known_type(slug: &str) -> bool {
    let s = normalize(slug.trim());
    let s = s.strip_prefix("type=").unwrap_or(&s).trim().to_string();
    catalog_slugs().contains(&s) || index().type_index.contains_key(&s)
}

// ---------------------------------------------------------------------------
// קטלוג סוגי השלוחות
// ---------------------------------------------------------------------------

const CATALOG_FILE: &str = "כל סוגי השלוחות הקיימות.txt";

/// מיפוי ידני: קטגוריה -> סלאגים. סלאג שאינו מופיע כאן נופל לקטגוריה האחרונה.
const CATEGORY_MAP: &[(&str, &[&str])] = &[
    (
        "ניתוב ומענה",
        &[
            "routing",
            "routing_time",
            "routing_yemot",
            "routing_1800",
            "routing_ip",
            "go_to_folder",
            "go_to_folder_time",
            "go_to_folder_count",
            "go_to_folder_date",
            "go_to_folder_from_list_all_information",
            "hangup",
            "queue",
            "access_filter",
            "template_filter",
            "key_play_set_record_go_to_foldar",
            "folder_play_random",
            "block_id_type",
            "private_did",
            "private_did_customer",
            "nitoviya",
            "nitoviya_b_say_number",
            "tzintuk",
        ],
    ),
    (
        "תפריט וניווט",
        &[
            "menu",
            "menu_star",
            "menu_voice",
            "message_options_menu",
            "start_select_file",
        ],
    ),
    (
        "השמעה",
        &[
            "playfile",
            "playfile_time",
            "playdir_time",
            "play_and_return",
            "last_play",
            "music_on_hold",
            "toplay_time",
            "id_message",
            "id_list_message",
            "phone_play_list",
            "daily_message",
            "number_now_is",
            "say_hour_and_minute",
        ],
    ),
    (
        "הקלטה וקליטת נתונים",
        &[
            "record",
            "recording_and_entering_data",
            "record_system_messages",
            "stt_dir_all_file",
            "say_amount_subscribers",
            "amount_incoming_phone_numbers",
            "counter_calculator",
        ],
    ),
    (
        "תפוצה וזיהוי",
        &[
            "template_add_number",
            "template_remove_number",
            "yemot_dialer",
            "yemot_dialer_campaign_list",
            "yemot_dialer_campaign_start",
            "add_id_to_list",
            "remove_id_from_list",
            "members_active",
        ],
    ),
    (
        "מסרים יוצאים",
        &[
            "sending_sms",
            "sending_sms_and_return",
            "send_fax",
            "recv_fax",
            "voicemail_email",
            "oref_alerts",
            "oref_alerts_quiet_wave",
        ],
    ),
    (
        "לימוד וניקוד",
        &[
            "points_save",
            "points_edit",
            "points_say_total",
            "points_to_other_id",
            "points_and_rating",
            "chapter_receive",
            "chapter_receive_mishnayot_select",
            "study_tracking",
            "tehillim_select_save_and_listen",
        ],
    ),
    (
        "מסחר וסליקה",
        &[
            "nedarim_plus",
            "pelecard",
            "cardcom",
            "credit_card",
            "sale_seats",
            "sale_products",
            "donation_campaign",
            "donation_campaign_some_actions",
            "fundraising",
            "coupon",
            "exchange_rates",
            "calculator_add_percentage",
        ],
    ),
    (
        "בחינות וסקרים",
        &[
            "examination",
            "examination_american",
            "seker",
            "trivia_questions",
            "approval_number",
        ],
    ),
    (
        "זמן ותאריך",
        &["zmanim", "time_keeper", "limit_call_seconds"],
    ),
    ("ועידה ושידור", &["confbridge", "conference_amount"]),
    (
        // קטגוריית ברירת מחדל — כל סלאג לא ממופה נופל לכאן
        "מערכת וניהול",
        &[
            "admin_login",
            "api",
            "create_ivr2",
            "open_wap_reseller_customer",
            "checking_units",
            "tasks_report",
        ],
    ),
];

static CATALOG_SLUGS: OnceLock<Vec<String>> = OnceLock::new();
static CATALOG: OnceLock<String> = OnceLock::new();

/// חילוץ הסלאגים מקובץ רשימת סוגי השלוחות, סובלני לשני הפורמטים:
/// `- [menu = תפריט](...)` וגם `- type=menu — תיאור`.
fn catalog_slugs() -> &'static Vec<String> {
    CATALOG_SLUGS.get_or_init(|| {
        let mut out: Vec<String> = Vec::new();
        let Some(f) = KNOWLEDGE_DIR.get_file(CATALOG_FILE) else {
            return out;
        };
        let text = std::str::from_utf8(f.contents()).unwrap_or("");
        for line in text.lines() {
            let t = line.trim();
            let Some(rest) = t.strip_prefix("- ") else {
                continue;
            };
            let rest = rest.trim_start_matches('[').trim_start();
            // "type=slug" או "type = slug"
            let cand = if let Some(after) = rest.strip_prefix("type") {
                after.trim_start().strip_prefix('=').map(|s| s.trim_start())
            } else {
                Some(rest)
            };
            let Some(cand) = cand else { continue };
            let slug: String = cand
                .chars()
                .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')
                .collect();
            if slug.is_empty() || slug == "type" {
                continue;
            }
            // חייב להופיע אחריו '=' או ' —' כדי לא לתפוס טקסט חופשי
            let after = cand[slug.len()..].trim_start();
            if !(after.starts_with('=') || after.starts_with('—') || after.starts_with('-')) {
                continue;
            }
            if !out.contains(&slug) {
                out.push(slug);
            }
        }
        out.sort();
        out
    })
}

/// קטלוג סוגי השלוחות — בלוק עברי יציב-בייטים, ממוין וקבוצתי.
// עדיין לא נצרך בתוך ה-crate — לולאת הסוכן (agent/) נבנית בנפרד.
#[allow(dead_code)]
pub fn type_catalog() -> &'static str {
    CATALOG.get_or_init(|| {
        let slugs = catalog_slugs();
        let mut placed: HashSet<&str> = HashSet::new();
        let mut lines: Vec<String> = Vec::new();
        lines.push(
            "סוגי שלוחות (type=). בחר מהרשימה בלבד; לפרטים חפש במאגר הידע.".to_string(),
        );

        let last = CATEGORY_MAP.len() - 1;
        for (ci, (cat, members)) in CATEGORY_MAP.iter().enumerate() {
            let mut group: Vec<&str> = slugs
                .iter()
                .map(|s| s.as_str())
                .filter(|s| {
                    if placed.contains(s) {
                        return false;
                    }
                    members.contains(s) || (ci == last)
                })
                .collect();
            group.sort_unstable();
            for s in &group {
                placed.insert(s);
            }
            if group.is_empty() {
                continue;
            }
            lines.push(format!("{}: {}", cat, group.join(", ")));
        }
        lines.join("\n")
    })
}

// ---------------------------------------------------------------------------
// בדיקות
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;


    /// תמונת מצב קפואה של צינור הנרמול+הטוקניזציה. שינוי כאן פירושו שינוי
    /// באינדקס — יש לוודא שגם `build.rs` נבנה מחדש (אותו קובץ מקור).
    #[test]
    fn normalize_tokenize_snapshot() {
        let sample = "## **תַּפְרִיט רָאשִׁי** — type=menu_star, מהשלוחות 12 ו-ID_Message\n\
                      ךםןףץ | api.key-name = Value_7";
        let norm = normalize(sample);
        let toks = tokenize(&norm).join("|");
        assert_eq!(
            toks,
            "תפריט|ראשי|type|menu_star|מהשלוחות|12|ו|id_message|כמנפצ|api|key|name|value_7"
        );
        assert_eq!(normalize(&norm), norm, "normalize must be idempotent");
    }

    /// שפיות: מספר הנתחים שנקרא מהאינדקס שווה למספר שכללי החלוקה
    /// המשותפים (`knowledge_text`) מייצרים כעת מאותם קבצים.
    #[test]
    fn build_time_chunk_count_matches_query_side() {
        let idx = index();
        let mut expected = 0usize;
        for name in &idx.files {
            let f = KNOWLEDGE_DIR
                .get_file(name)
                .unwrap_or_else(|| panic!("missing embedded file {}", name));
            let text = std::str::from_utf8(f.contents()).expect("utf8");
            expected += knowledge_text::chunk_count(text);
        }
        assert_eq!(
            expected,
            idx.chunks.len(),
            "chunking rules drifted from the built index"
        );
    }

    #[test]
    fn normalization_strips_nikud_and_finals() {
        assert_eq!(normalize("שָׁלוֹם"), "שלומ");
        assert_eq!(normalize("ךםןףץ"), "כמנפצ");
        assert_eq!(normalize("Menu=MENU"), "menu=menu");
        // אידמפוטנטי
        let n = normalize("תַּפְרִיט ראשי");
        assert_eq!(normalize(&n), n);
    }

    #[test]
    fn tokenizer_splits_expected_runs() {
        let t = tokenize(&normalize("type=menu_star 12 שלוחה, ניתוב-שיחה"));
        assert_eq!(
            t,
            vec!["type", "menu_star", "12", "שלוחה", "ניתוב", "שיחה"]
        );
    }

    #[test]
    fn clitic_expansion() {
        assert!(expand_clitics("כתפריט").contains(&"תפריט".to_string()));
        assert!(expand_clitics("ושלוחה").contains(&"שלוחה".to_string()));
        assert!(expand_clitics("מהשלוחה").contains(&"שלוחה".to_string()));
        // לא מרחיבים כשנשארות פחות מ-2 אותיות
        assert_eq!(expand_clitics("של"), vec!["של".to_string()]);
        // לא נוגעים בלטינית
        assert_eq!(expand_clitics("menu"), vec!["menu".to_string()]);
    }

    #[test]
    fn estimate_tokens_rounds_up() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abc"), 2); // 3/2.44 = 1.23 -> 2
        assert_eq!(estimate_tokens("שלום עולם"), 4);
    }

    #[test]
    fn index_loads_and_chunks_are_sane() {
        let idx = index();
        assert!(idx.files.len() >= 100, "files: {}", idx.files.len());
        assert!(idx.chunks.len() > idx.files.len());
        assert!(!idx.postings.is_empty());
        assert!(idx.avgdl > 1.0);
        for ch in &idx.chunks {
            assert!(ch.end >= ch.start);
            assert!(ch.part >= 1 && ch.part <= ch.nparts);
        }
        // כל נתח ניתן לחיתוך על גבול תווים תקין
        for ci in 0..idx.chunks.len().min(500) {
            let _ = chunk_text(idx, ci as u32);
        }
    }

    #[test]
    fn chunk_boundaries_cover_each_file_contiguously() {
        let idx = index();
        for (fi, name) in idx.files.iter().enumerate() {
            let len = KNOWLEDGE_DIR.get_file(name).unwrap().contents().len() as u32;
            let mut prev_end = 0u32;
            let mut any = false;
            for ch in idx.chunks.iter().filter(|c| c.file == fi as u32) {
                assert_eq!(ch.start, prev_end, "gap/overlap in {}", name);
                prev_end = ch.end;
                any = true;
            }
            assert!(any, "no chunks for {}", name);
            assert_eq!(prev_end, len, "chunks don't cover {}", name);
        }
    }

    #[test]
    fn chunks_respect_max_size() {
        let idx = index();
        for ci in 0..idx.chunks.len() {
            let t = chunk_text(idx, ci as u32);
            assert!(
                estimate_tokens(t) <= 1500,
                "chunk {} too big: {}",
                ci,
                estimate_tokens(t)
            );
        }
    }

    #[test]
    fn bm25_ranks_menu_chunk_first() {
        let out = search_knowledge(
            "תגדיר שלוחה 3 כתפריט עם 2 אפשרויות",
            SearchOpts {
                top_k: 3,
                ..Default::default()
            },
        );
        let first = out.lines().next().unwrap_or("");
        assert!(
            first.contains("תפריט"),
            "expected a תפריט chunk first, got: {}",
            first
        );
    }

    #[test]
    fn catalog_is_complete_and_deterministic() {
        let a = type_catalog();
        let b = type_catalog();
        assert_eq!(a, b);
        assert!(a.starts_with("סוגי שלוחות (type=). בחר מהרשימה בלבד"));
        let slugs = catalog_slugs();
        assert!(slugs.len() >= 99, "only {} slugs parsed", slugs.len());
        for s in slugs {
            assert!(a.contains(s.as_str()), "slug {} missing from catalog", s);
        }
        assert!(estimate_tokens(a) < 900, "catalog too big: {}", estimate_tokens(a));
    }

    #[test]
    fn known_type_and_param() {
        assert!(is_known_type("menu"));
        assert!(is_known_type("type=playfile"));
        assert!(!is_known_type("no_such_type_xyz"));
        assert!(is_known_param("type"));
        assert!(!is_known_param("no_such_param_xyz"));
    }

    #[test]
    fn budget_is_enforced() {
        for budget in [400usize, 1200, 3000] {
            let out = search_knowledge(
                "תפריט",
                SearchOpts {
                    top_k: 10,
                    max_tokens: budget,
                    ..Default::default()
                },
            );
            // סכום הטוקנים המדווח בכותרות התוצאות אינו חורג מהתקציב
            let mut sum = 0usize;
            for l in out.lines() {
                if let Some(p) = l.rfind("(") {
                    if l.trim_end().ends_with("tok)") {
                        let n: usize = l[p + 1..]
                            .trim_end_matches(" tok)")
                            .trim()
                            .parse()
                            .unwrap_or(0);
                        sum += n;
                    }
                }
            }
            assert!(sum <= budget, "budget {} exceeded: {}", budget, sum);
        }
    }

    #[test]
    fn cursor_advances() {
        let a = search_knowledge("תפריט", SearchOpts::default());
        let cur = a
            .lines()
            .last()
            .and_then(|l| l.split("cursor=\"").nth(1))
            .map(|s| s.trim_end_matches('"').to_string());
        assert!(cur.is_some(), "no cursor in: {}", a.lines().last().unwrap());
        let b = search_knowledge(
            "תפריט",
            SearchOpts {
                cursor: cur,
                ..Default::default()
            },
        );
        assert_ne!(a, b);
    }

    #[test]
    fn section_toc_and_unknown_file() {
        let idx = index();
        let name = idx.files[0].strip_suffix(".txt").unwrap().to_string();
        let toc = get_knowledge_section(&name, None, None).unwrap();
        assert!(toc.contains("תוכן עניינים"));
        let err = get_knowledge_section("קובץ שלא קיים בכלל", None, None).unwrap_err();
        assert!(err.contains("אולי התכוונת"));
    }

    #[test]
    fn lookup_param_returns_occurrences() {
        let out = lookup_param("type");
        assert!(out.contains("מופעים מתועדים"));
        assert!(estimate_tokens(&out) <= 3200);
        assert!(lookup_param("no_such_param_xyz").contains("לא נמצאה"));
    }

    #[test]
    fn oversize_chunk_is_cut_line_by_line_with_next() {
        // תקציב זעיר מכריח החזרה שורה-שורה של נתח בודד
        let out = search_knowledge(
            "תפריט",
            SearchOpts {
                top_k: 5,
                max_tokens: 200,
                ..Default::default()
            },
        );
        assert!(out.starts_with("[1] "), "{}", out);
        let body: usize = out
            .lines()
            .filter(|l| !l.starts_with('[') && !l.starts_with("---") && !l.starts_with("נמצאו"))
            .map(estimate_tokens)
            .sum();
        assert!(body <= 220, "body {} over budget", body);
    }
}
