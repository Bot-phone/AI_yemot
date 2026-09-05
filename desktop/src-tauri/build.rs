//! Build script: Tauri codegen + build-time lexical (BM25) index over the
//! embedded Hebrew knowledge base in `knowledge/`.
//!
//! The index is written as a compact hand-rolled varint binary to
//! `$OUT_DIR/knowledge_index.bin` and pulled into the binary by
//! `knowledge.rs` via `include_bytes!`. No extra crate dependencies.
//!
//! The normalization/tokenization/chunking/ini rules are NOT defined here:
//! they live in `src/knowledge_text.rs`, which is included both by this build
//! script and by `src/knowledge.rs` (query side), so the index and the queries
//! can never drift apart.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[path = "src/knowledge_text.rs"]
mod knowledge_text;

use knowledge_text::{
    extract_key, extract_kv, hard_split, normalize, split_sections, term_variants, tokenize,
};

// ---------------------------------------------------------------------------
// Chunking
// ---------------------------------------------------------------------------

struct RawChunk {
    file: u32,
    start: usize,
    end: usize,
    heading: String,
    part: u16,
    nparts: u16,
}

// ---------------------------------------------------------------------------
// Varint writer
// ---------------------------------------------------------------------------

fn put_uvar(buf: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            buf.push(b);
            break;
        }
        buf.push(b | 0x80);
    }
}

fn put_str(buf: &mut Vec<u8>, s: &str) {
    put_uvar(buf, s.len() as u64);
    buf.extend_from_slice(s.as_bytes());
}

// ---------------------------------------------------------------------------

struct Posting {
    chunk: u32,
    tf: u16,
    in_heading: bool,
}

fn main() {
    let t0 = std::time::Instant::now();
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    // מאגר יחיד בשורש המאגר (repo root) — אין עותק כפול תחת src-tauri.
    let kdir = manifest.join("..").join("..").join("knowledge");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/knowledge_text.rs");
    let kdir = fs::canonicalize(&kdir).unwrap_or(kdir);
    println!("cargo:rerun-if-changed={}", kdir.display());

    let mut files: Vec<(String, String)> = Vec::new();
    let mut entries: Vec<PathBuf> = fs::read_dir(&kdir)
        .expect("knowledge dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|e| e == "txt").unwrap_or(false))
        .collect();
    entries.sort();
    for p in &entries {
        println!("cargo:rerun-if-changed={}", p.display());
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let text = fs::read_to_string(p).unwrap_or_default();
        files.push((name, text));
    }

    // --- chunk ---
    let mut chunks: Vec<RawChunk> = Vec::new();
    for (fi, (_name, text)) in files.iter().enumerate() {
        for (s, e, h) in split_sections(text) {
            let parts = hard_split(text, s, e);
            let np = parts.len() as u16;
            for (pi, (ps, pe)) in parts.into_iter().enumerate() {
                chunks.push(RawChunk {
                    file: fi as u32,
                    start: ps,
                    end: pe,
                    heading: h.clone(),
                    part: (pi + 1) as u16,
                    nparts: np,
                });
            }
        }
    }

    // --- postings, type/param indexes ---
    let mut postings: HashMap<String, Vec<Posting>> = HashMap::new();
    let mut doc_lens: Vec<u32> = Vec::with_capacity(chunks.len());
    let mut type_index: HashMap<String, Vec<(u32, bool)>> = HashMap::new();
    let mut param_index: HashMap<String, Vec<u32>> = HashMap::new();

    let h1_of_file: Vec<String> = files
        .iter()
        .map(|(_, t)| {
            normalize(
                t.lines()
                    .find(|l| l.starts_with("# "))
                    .unwrap_or("")
                    .trim_start_matches("# "),
            )
        })
        .collect();

    for (ci, ch) in chunks.iter().enumerate() {
        let ci32 = ci as u32;
        let body = &files[ch.file as usize].1[ch.start..ch.end];
        let fname = &files[ch.file as usize].0;
        let title = fname.strip_suffix(".txt").unwrap_or(fname);
        let head_toks = tokenize(&normalize(&ch.heading));
        let body_toks = tokenize(&normalize(body));
        doc_lens.push((head_toks.len() + body_toks.len()) as u32);

        // כל מונח מאונדקס גם בצורת השטח וגם בווריאנטים שלו (אותיות שימוש +
        // גזע ללא סיומת), כדי שהנרמול העברי יהיה סימטרי עם צד השאילתה.
        let mut tf: HashMap<String, (u32, bool)> = HashMap::new();
        for t in &head_toks {
            for v in term_variants(t) {
                let e = tf.entry(v).or_insert((0, false));
                e.0 += 1;
                e.1 = true;
            }
        }
        for t in &body_toks {
            for v in term_variants(t) {
                let e = tf.entry(v).or_insert((0, false));
                e.0 += 1;
            }
        }
        for (t, (n, inh)) in tf {
            postings.entry(t).or_default().push(Posting {
                chunk: ci32,
                tf: n.min(u16::MAX as u32) as u16,
                in_heading: inh,
            });
        }

        // type= / key= scanning
        let nheading = normalize(&ch.heading);
        let mut in_ini = false;
        for line in body.lines() {
            let lt = line.trim();
            if knowledge_text::track_ini_fence(lt, &mut in_ini) {
                continue;
            }
            // type= anywhere (ini blocks and prose "type=" lines)
            if let Some(slug) = extract_kv(lt, "type") {
                let declaring = nheading.contains(&slug)
                    || normalize(title).contains(&slug)
                    || h1_of_file[ch.file as usize].contains(&slug);
                let e = type_index.entry(slug).or_default();
                if !e.iter().any(|(c, _)| *c == ci32) {
                    e.push((ci32, declaring));
                }
            }
            if in_ini {
                if let Some(key) = extract_key(lt) {
                    let e = param_index.entry(key).or_default();
                    if !e.contains(&ci32) {
                        e.push(ci32);
                    }
                }
            }
        }
    }

    // מיון רשימות ה-type: הנתח שמצהיר על הסוג קודם.
    // היוריסטיקה: כותרת שמכילה את הסלאג > נתח שמצהיר על מעט סוגים
    // (נתח קטלוג מכיל עשרות סוגים ולכן יורד למטה) > סדר האינדקס.
    let mut types_per_chunk: HashMap<u32, usize> = HashMap::new();
    for v in type_index.values() {
        for (c, _) in v {
            *types_per_chunk.entry(*c).or_insert(0) += 1;
        }
    }
    let mut type_index: Vec<(String, Vec<u32>)> = type_index
        .into_iter()
        .map(|(k, mut v)| {
            v.sort_by_key(|(c, declaring)| {
                (
                    u8::from(!*declaring),
                    *types_per_chunk.get(c).unwrap_or(&0),
                    *c,
                )
            });
            (k, v.into_iter().map(|(c, _)| c).collect())
        })
        .collect();
    type_index.sort();
    let mut param_index: Vec<(String, Vec<u32>)> = param_index.into_iter().collect();
    param_index.sort();

    // --- serialize ---
    let mut buf: Vec<u8> = Vec::with_capacity(4 << 20);
    buf.extend_from_slice(b"KIX1");
    put_uvar(&mut buf, files.len() as u64);
    for (name, _) in &files {
        put_str(&mut buf, name);
    }
    put_uvar(&mut buf, chunks.len() as u64);
    for (i, ch) in chunks.iter().enumerate() {
        put_uvar(&mut buf, ch.file as u64);
        put_uvar(&mut buf, ch.start as u64);
        put_uvar(&mut buf, (ch.end - ch.start) as u64);
        put_str(&mut buf, &ch.heading);
        put_uvar(&mut buf, ch.part as u64);
        put_uvar(&mut buf, ch.nparts as u64);
        put_uvar(&mut buf, doc_lens[i] as u64);
    }
    let mut terms: Vec<(String, Vec<Posting>)> = postings.into_iter().collect();
    terms.sort_by(|a, b| a.0.cmp(&b.0));
    put_uvar(&mut buf, terms.len() as u64);
    for (t, mut ps) in terms {
        put_str(&mut buf, &t);
        ps.sort_by_key(|p| p.chunk);
        put_uvar(&mut buf, ps.len() as u64);
        let mut prev = 0u32;
        for p in ps {
            put_uvar(&mut buf, (p.chunk - prev) as u64);
            prev = p.chunk;
            put_uvar(&mut buf, p.tf as u64);
            buf.push(u8::from(p.in_heading));
        }
    }
    for idx in [&type_index, &param_index] {
        put_uvar(&mut buf, idx.len() as u64);
        for (k, v) in idx {
            put_str(&mut buf, k);
            put_uvar(&mut buf, v.len() as u64);
            for c in v {
                put_uvar(&mut buf, *c as u64);
            }
        }
    }

    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("knowledge_index.bin");
    write_if_changed(&out, &buf);
    println!(
        "cargo:warning=knowledge index: {} files, {} chunks, {} bytes, built in {} ms",
        files.len(),
        chunks.len(),
        buf.len(),
        t0.elapsed().as_millis()
    );

    tauri_build::build()
}

fn write_if_changed(path: &Path, data: &[u8]) {
    if let Ok(old) = fs::read(path) {
        if old == data {
            return;
        }
    }
    fs::write(path, data).expect("write index");
}
