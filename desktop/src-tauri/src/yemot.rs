//! Yemot HaMashiach API client.
//!
//! Everything goes through one shared `reqwest::Client` and one
//! [`YemotClient`], which holds the session token privately (it is sent in the
//! `authorization` header, never logged, never returned to the frontend or to
//! the model) and caches `ext.ini` reads for a short while.
#![allow(dead_code)]

use crate::yemot_ini::{ExtIni, IniLine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, Semaphore};

const YEMOT_API_BASE: &str = "https://www.call2all.co.il/ym/api/";
const CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_CHILD_REQUESTS: usize = 25;
/// Concurrent child reads in `list_extensions(depth=2)`.
const MAX_PARALLEL_READS: usize = 6;
const MAX_FILE_LINES: usize = 40;
const MAX_TREE_LINES: usize = 60;
const DEFAULT_MAX_BYTES: usize = 8192;
const NOTE_UNVERIFIED: &str = "לא ניתן היה לאמת את השינוי (הקריאה החוזרת נכשלה)";

/// Extensions we refuse to read as text (audio / binary).
const BINARY_EXTS: &[&str] = &[
    "wav", "mp3", "wma", "gsm", "ogg", "opus", "m4a", "aac", "amr", "zip", "rar", "gz", "png",
    "jpg", "jpeg", "gif", "bmp", "pdf", "exe", "dll", "bin",
];

// ---------------------------------------------------------------------------
// Shared HTTP client
// ---------------------------------------------------------------------------

static HTTP: OnceLock<reqwest::Client> = OnceLock::new();

/// The one and only HTTP client used by this module.
pub fn http() -> &'static reqwest::Client {
    HTTP.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(20))
            .user_agent("AI-Yemot-Desktop")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YemotError {
    /// Session exists but has not passed two-factor verification.
    MfaRequired,
    /// Token invalid / expired / not logged in.
    SessionExpired(String),
    /// `responseStatus=FORBIDDEN`.
    Forbidden(String),
    /// Rejected locally (bad path, bad value) or malformed server reply.
    BadRequest(String),
    /// `responseStatus=ERROR` / `EXCEPTION`.
    Api { code: Option<i64>, message: String },
    /// Transport-level failure — the request may or may not have been applied.
    Network(String),
}

impl fmt::Display for YemotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&render_error(self))
    }
}

impl std::error::Error for YemotError {}

/// Model-facing one-line rendering of an error, with a stable code prefix.
pub fn render_error(err: &YemotError) -> String {
    match err {
        YemotError::MfaRequired => "SESSION_EXPIRED: נדרש אימות דו-שלבי (MFA_REQUIRED). \
             Stop and wait — do not retry."
            .to_string(),
        YemotError::SessionExpired(m) => {
            format!("SESSION_EXPIRED: {} Stop and wait — do not retry.", m)
        }
        YemotError::Forbidden(m) => format!("FORBIDDEN: {}", m),
        YemotError::BadRequest(m) => format!("BAD_REQUEST: {}", m),
        YemotError::Api { code, message } => {
            let code = code.map(|c| c.to_string()).unwrap_or_else(|| "-".to_string());
            format!("ERROR[{}]: {}", code, message)
        }
        YemotError::Network(m) => format!(
            "NETWORK_ERROR: {} — the request may or may not have been applied; \
             read the extension config to verify before retrying.",
            m
        ),
    }
}

fn is_missing_file(err: &YemotError) -> bool {
    let msg = match err {
        YemotError::Api { message, .. } => message.to_lowercase(),
        YemotError::BadRequest(m) => m.to_lowercase(),
        _ => return false,
    };
    msg.contains("does not exist")
        || msg.contains("not exist")
        || msg.contains("no such file")
        || msg.contains("file not found")
}

fn looks_like_dead_session(message: &str) -> bool {
    let m = message.to_lowercase();
    (m.contains("token") || m.contains("session"))
        && (m.contains("invalid")
            || m.contains("expire")
            || m.contains("incorrect")
            || m.contains("not found")
            || m.contains("wrong"))
        || m.contains("not logged in")
        || m.contains("no session")
}

/// Classify a decoded API response by its `responseStatus` / `message`.
pub fn classify_response(json: &Value) -> Result<(), YemotError> {
    let status = json
        .get("responseStatus")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let message = json
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let code = json.get("messageCode").and_then(|v| v.as_i64());

    // MFA_REQUIRED arrives as ERROR *or* FORBIDDEN.
    if message.contains("MFA_REQUIRED") {
        return Err(YemotError::MfaRequired);
    }

    match status {
        "OK" => Ok(()),
        "FORBIDDEN" => Err(YemotError::Forbidden(if message.is_empty() {
            "הבקשה נדחתה על ידי השרת".to_string()
        } else {
            message
        })),
        "ERROR" | "EXCEPTION" => {
            if looks_like_dead_session(&message) {
                Err(YemotError::SessionExpired(message))
            } else {
                Err(YemotError::Api {
                    code,
                    message: if message.is_empty() {
                        format!("הפעולה נכשלה ({})", status)
                    } else {
                        message
                    },
                })
            }
        }
        "" => Err(YemotError::BadRequest(
            "תשובה לא תקינה מהשרת (חסר responseStatus)".to_string(),
        )),
        other => Err(YemotError::Api {
            code,
            message: format!("סטטוס לא מוכר: {} {}", other, message).trim_end().to_string(),
        }),
    }
}

// ---------------------------------------------------------------------------
// Path canonicalisation
// ---------------------------------------------------------------------------

fn strip_scheme(input: &str) -> &str {
    let s = input.trim();
    for p in ["ivr2:", "ivr2/", "ivr:", "ivr/"] {
        if let Some(rest) = s.strip_prefix(p) {
            return rest;
        }
    }
    s
}

/// Canonicalise an extension path to `ivr2:/1/2` (root → `ivr2:/`).
///
/// Accepts `/1/2`, `1/2`, `ivr2:/1/2`, `ivr2:1/2`, with or without a trailing
/// slash. Anything containing characters outside `[0-9/]` is rejected.
pub fn canon_ext(input: &str) -> Result<String, YemotError> {
    let body = strip_scheme(input).trim();
    let body = body.trim_matches('/');
    if body.is_empty() {
        return Ok("ivr2:/".to_string());
    }
    let mut segments = Vec::new();
    for seg in body.split('/') {
        if seg.is_empty() {
            return Err(YemotError::BadRequest(format!(
                "נתיב שלוחה לא תקין: {}",
                input
            )));
        }
        if !seg.bytes().all(|b| b.is_ascii_digit()) {
            return Err(YemotError::BadRequest(format!(
                "נתיב שלוחה לא תקין (מותרים ספרות ו-/ בלבד): {}",
                input
            )));
        }
        segments.push(seg);
    }
    Ok(format!("ivr2:/{}", segments.join("/")))
}

/// Characters a file name may never contain (path separators, wildcards and
/// the Windows-reserved set). Everything else — Hebrew included — is allowed:
/// an allow-list of ASCII would reject legitimate names like `רשימה.txt`.
const FILENAME_DENY: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

fn is_valid_filename(name: &str) -> bool {
    if name.is_empty() || name.starts_with('.') || name.contains("..") {
        return false;
    }
    if name.chars().any(|c| FILENAME_DENY.contains(&c) || c.is_control()) {
        return false;
    }
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return false;
    };
    !stem.trim().is_empty() && !ext.trim().is_empty()
}

/// Canonicalise a file path (`ivr2:/1/ext.ini`). The directory part must be a
/// valid extension path and the last segment a plain `name.ext` filename.
pub fn canon_file(input: &str) -> Result<String, YemotError> {
    let body = strip_scheme(input).trim().trim_start_matches('/');
    let (dir, name) = match body.rsplit_once('/') {
        Some((d, n)) => (d, n),
        None => ("", body),
    };
    if !is_valid_filename(name) {
        return Err(YemotError::BadRequest(format!(
            "שם קובץ לא תקין: {}",
            input
        )));
    }
    let dir = canon_ext(dir)?;
    Ok(join_file(&dir, name))
}

fn join_file(canon_dir: &str, name: &str) -> String {
    if canon_dir == "ivr2:/" {
        format!("ivr2:/{}", name)
    } else {
        format!("{}/{}", canon_dir, name)
    }
}

fn ext_ini_path(canon_dir: &str) -> String {
    join_file(canon_dir, "ext.ini")
}

/// `ivr2:/1/2` → `/1/2` (what the model and the UI see).
pub fn display_path(canon: &str) -> String {
    let s = canon.strip_prefix("ivr2:").unwrap_or(canon);
    if s.is_empty() {
        "/".to_string()
    } else {
        s.to_string()
    }
}

fn parent_of_file(canon_file_path: &str) -> String {
    match canon_file_path.rsplit_once('/') {
        Some(("ivr2:", _)) | None => "ivr2:/".to_string(),
        Some((dir, _)) => dir.to_string(),
    }
}

fn file_extension(path: &str) -> String {
    path.rsplit('/')
        .next()
        .and_then(|n| n.rsplit_once('.'))
        .map(|(_, e)| e.to_lowercase())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Value types
// ---------------------------------------------------------------------------

/// Where an `ext.ini` snapshot came from.
///
/// `Listing` entries are the `extIni` object of a `GetIVR2Dir` reply: handy for
/// a tree view, but they carry no size/mtime and the server may summarise them,
/// so they must never satisfy the read-before-write gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtSource {
    File,
    Listing,
}

#[derive(Debug, Clone)]
pub struct ExtRead {
    pub exists: bool,
    pub ini: ExtIni,
    pub size: Option<u64>,
    pub mtime: Option<String>,
    /// `File` = read from `ext.ini` itself; `Listing` = seeded from a directory.
    pub source: ExtSource,
}

/// Stable fingerprint of one file's state (existence + contents). Used to
/// detect a server-side change between proposal and approval, and between an
/// apply and its undo.
///
/// FNV-1a: no dependency, and a collision here only costs a refused write.
pub fn content_hash(exists: bool, text: &str) -> String {
    let text = if exists { text } else { "" };
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("{}:{:x}", exists, hash)
}

impl ExtRead {
    /// Stable fingerprint of the file as it was when this snapshot was taken.
    pub fn snapshot_hash(&self) -> String {
        content_hash(self.exists, &self.ini.to_text())
    }
}

/// One file inside an extension folder (`list_files`).
#[derive(Debug, Clone, Serialize)]
pub struct FileEntry {
    pub name: String,
    pub size: Option<u64>,
    pub mtime: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtNode {
    pub path: String,
    pub ext_type: String,
    pub title: String,
}

#[derive(Debug, Clone)]
pub struct TextFile {
    pub exists: bool,
    pub contents: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SystemInfo {
    pub system: String,
    pub units: Option<f64>,
    pub units_expire: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParamOutcome {
    pub key: String,
    pub value: String,
    pub applied: bool,
    pub note: Option<String>,
}

// ---------------------------------------------------------------------------
// Session cache
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct CachedExt {
    exists: bool,
    ini: ExtIni,
    size: Option<u64>,
    mtime: Option<String>,
    source: ExtSource,
    at: Instant,
}

/// Per-session cache of `ext.ini` reads, keyed by canonical extension path.
#[derive(Debug, Default)]
pub struct SessionCache {
    ext: HashMap<String, CachedExt>,
}

impl SessionCache {
    fn fresh(&self, path: &str) -> Option<&CachedExt> {
        self.ext.get(path).filter(|e| e.at.elapsed() < CACHE_TTL)
    }

    fn put(&mut self, path: &str, entry: CachedExt) {
        self.ext.insert(path.to_string(), entry);
    }

    fn invalidate(&mut self, path: &str) {
        self.ext.remove(path);
    }

    fn clear(&mut self) {
        self.ext.clear();
    }
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// A Yemot API session. The token never leaves this struct.
pub struct YemotClient {
    token: String,
    cache: RwLock<SessionCache>,
}

impl fmt::Debug for YemotClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("YemotClient")
            .field("token", &"<redacted>")
            .finish_non_exhaustive()
    }
}

/// One raw API request: decodes the JSON body without classifying it.
async fn post_json(
    token: &str,
    endpoint: &str,
    params: Map<String, Value>,
) -> Result<Value, YemotError> {
    let url = format!("{}{}", YEMOT_API_BASE, endpoint);
    let res = http()
        .post(&url)
        .header("authorization", token)
        .header("Content-Type", "application/json")
        .json(&Value::Object(params))
        .send()
        .await
        // `without_url` keeps the token-bearing URL out of the message.
        .map_err(|e| YemotError::Network(e.without_url().to_string()))?;

    let status = res.status();
    let body = res
        .text()
        .await
        .map_err(|e| YemotError::Network(e.without_url().to_string()))?;

    let json: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => {
            if status.as_u16() == 401 || status.as_u16() == 403 {
                return Err(YemotError::Forbidden(format!("HTTP {}", status.as_u16())));
            }
            return Err(YemotError::BadRequest(format!(
                "תשובה לא תקינה מהשרת (HTTP {})",
                status.as_u16()
            )));
        }
    };

    Ok(json)
}

/// One API request, classified. Free function so it can be spawned onto a task.
async fn call_raw(
    token: &str,
    endpoint: &str,
    params: Map<String, Value>,
) -> Result<Value, YemotError> {
    let json = post_json(token, endpoint, params).await?;
    classify_response(&json)?;
    Ok(json)
}

/// Delay before the single retry of a failed READ.
const READ_RETRY_DELAY: Duration = Duration::from_millis(500);

/// A READ, retried once on a transport failure. Never use this for a write:
/// a `NETWORK_ERROR` on `UpdateExtension` may already have been applied.
async fn call_raw_read(
    token: &str,
    endpoint: &str,
    params: Map<String, Value>,
) -> Result<Value, YemotError> {
    match call_raw(token, endpoint, params.clone()).await {
        Err(YemotError::Network(_)) => {
            tokio::time::sleep(READ_RETRY_DELAY).await;
            call_raw(token, endpoint, params).await
        }
        other => other,
    }
}

fn params_of(pairs: &[(&str, &str)]) -> Map<String, Value> {
    let mut m = Map::new();
    for (k, v) in pairs {
        m.insert((*k).to_string(), json!(v));
    }
    m
}

fn ini_from_object(obj: &Map<String, Value>) -> ExtIni {
    let lines = obj
        .iter()
        .map(|(k, v)| {
            let value = match v {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            };
            IniLine::Pair {
                key: k.clone(),
                value: value.clone(),
                raw: format!("{}={}", k, value),
            }
        })
        .collect();
    ExtIni { lines }
}

/// Sort key for an extension path: `/10` after `/2`, never lexically.
pub fn natural_key(path: &str) -> Vec<u64> {
    display_path(path)
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<u64>().unwrap_or(u64::MAX))
        .collect()
}

fn children_from_dir(parent_canon: &str, json: &Value) -> Vec<ExtNode> {
    let mut out = Vec::new();
    let Some(dirs) = json.get("dirs").and_then(|v| v.as_array()) else {
        return out;
    };
    for d in dirs {
        let Some(name) = d.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let child = if parent_canon == "ivr2:/" {
            canon_ext(name)
        } else {
            canon_ext(&format!("{}/{}", display_path(parent_canon), name))
        };
        let Ok(path) = child else { continue };
        out.push(ExtNode {
            path,
            ext_type: d
                .get("extType")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            title: d
                .get("extTitle")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        });
    }
    out
}

/// Files of a `GetIVR2Dir` reply. `files` holds audio/other files; ini, system
/// messages and reports live in their own arrays, so all four are collected.
fn files_from_dir(json: &Value) -> Vec<FileEntry> {
    let mut out: Vec<FileEntry> = Vec::new();
    for bucket in ["files", "ini", "messages", "html"] {
        let Some(arr) = json.get(bucket).and_then(|v| v.as_array()) else {
            continue;
        };
        for f in arr {
            let Some(name) = f.get("name").and_then(|v| v.as_str()) else {
                continue;
            };
            if name.is_empty() || out.iter().any(|e| e.name == name) {
                continue;
            }
            out.push(FileEntry {
                name: name.to_string(),
                size: f.get("size").and_then(|v| v.as_u64()),
                mtime: f
                    .get("mtime")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Did the read-back value match what we asked for?
///
/// Two asymmetries of the Yemot format matter here: an empty value may come
/// back as a missing key (the server drops `key=`), and the ini parser trims
/// both sides, so a trailing space in the request is not a failed write.
fn param_applied(current: Option<&str>, wanted: &str) -> bool {
    let wanted = wanted.trim();
    match current {
        Some(c) => c.trim() == wanted,
        None => wanted.is_empty(),
    }
}

impl YemotClient {
    pub fn new(token: impl Into<String>) -> Self {
        YemotClient {
            token: token.into(),
            cache: RwLock::new(SessionCache::default()),
        }
    }

    async fn call(&self, endpoint: &str, params: Map<String, Value>) -> Result<Value, YemotError> {
        call_raw(&self.token, endpoint, params).await
    }

    /// Like [`Self::call`], but retried once on a transport failure. Reads only.
    async fn call_read(
        &self,
        endpoint: &str,
        params: Map<String, Value>,
    ) -> Result<Value, YemotError> {
        call_raw_read(&self.token, endpoint, params).await
    }

    /// Is a fresh `ext.ini` for this path already cached?
    pub async fn has_ext(&self, path: &str) -> bool {
        let Ok(canon) = canon_ext(path) else {
            return false;
        };
        self.cache.read().await.fresh(&canon).is_some()
    }

    /// Drop every cached `ext.ini`.
    pub async fn clear(&self) {
        self.cache.write().await.clear();
    }

    async fn cache_put(&self, canon: &str, read: &ExtRead) {
        self.cache.write().await.put(
            canon,
            CachedExt {
                exists: read.exists,
                ini: read.ini.clone(),
                size: read.size,
                mtime: read.mtime.clone(),
                source: read.source,
                at: Instant::now(),
            },
        );
    }

    /// Read (and cache) the `ext.ini` of an extension.
    /// A missing file is *not* an error — it yields `exists=false`.
    ///
    /// The result carries its [`ExtSource`]: a cache hit seeded from a directory
    /// listing is `Listing`, and callers that are about to write must insist on
    /// `File` (see [`Self::get_ext_ini_fresh`]).
    pub async fn get_ext_ini(&self, path: &str) -> Result<ExtRead, YemotError> {
        let canon = canon_ext(path)?;
        if let Some(hit) = self.cache.read().await.fresh(&canon) {
            return Ok(ExtRead {
                exists: hit.exists,
                ini: hit.ini.clone(),
                size: hit.size,
                mtime: hit.mtime.clone(),
                source: hit.source,
            });
        }
        self.fetch_ext_ini(&canon).await
    }

    /// Read `ext.ini` from the server, ignoring (and refreshing) the cache.
    /// The result is always `ExtSource::File`.
    pub async fn get_ext_ini_fresh(&self, path: &str) -> Result<ExtRead, YemotError> {
        let canon = canon_ext(path)?;
        self.fetch_ext_ini(&canon).await
    }

    async fn fetch_ext_ini(&self, canon: &str) -> Result<ExtRead, YemotError> {
        let what = ext_ini_path(canon);
        let json = match self
            .call_read("GetTextFile", params_of(&[("what", &what)]))
            .await
        {
            Ok(j) => j,
            Err(e) if is_missing_file(&e) => {
                let read = ExtRead {
                    exists: false,
                    ini: ExtIni::default(),
                    size: None,
                    mtime: None,
                    source: ExtSource::File,
                };
                self.cache_put(canon, &read).await;
                return Ok(read);
            }
            Err(e) => return Err(e),
        };

        let meta = json
            .get("file")
            .and_then(|f| f.as_array().and_then(|a| a.first()).or(Some(f)))
            .cloned()
            .unwrap_or(Value::Null);
        let exists = meta.get("exists").and_then(|v| v.as_bool()).unwrap_or(true);
        let contents = json.get("contents").and_then(|v| v.as_str()).unwrap_or("");

        let read = ExtRead {
            exists,
            ini: if exists {
                ExtIni::parse(contents)
            } else {
                ExtIni::default()
            },
            size: meta.get("size").and_then(|v| v.as_u64()),
            mtime: meta
                .get("mtime")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            source: ExtSource::File,
        };
        self.cache_put(canon, &read).await;
        Ok(read)
    }

    async fn seed_cache_from_dir(&self, canon: &str, json: &Value) {
        if let Some(obj) = json.get("extIni").and_then(|v| v.as_object()) {
            if obj.is_empty() {
                return;
            }
            // A real file read must not be downgraded to a listing snapshot.
            if matches!(
                self.cache.read().await.fresh(canon),
                Some(CachedExt { source: ExtSource::File, .. })
            ) {
                return;
            }
            let read = ExtRead {
                exists: true,
                ini: ini_from_object(obj),
                size: None,
                mtime: None,
                source: ExtSource::Listing,
            };
            self.cache_put(canon, &read).await;
        }
    }

    /// List child extensions of `path`. `depth = 2` also lists grandchildren
    /// (children fetched concurrently, capped at 25 requests).
    pub async fn list_extensions(
        &self,
        path: &str,
        depth: u8,
    ) -> Result<Vec<ExtNode>, YemotError> {
        let root = canon_ext(path)?;
        let json = self
            .call_read("GetIVR2Dir", params_of(&[("path", &root)]))
            .await?;
        self.seed_cache_from_dir(&root, &json).await;
        let mut nodes = children_from_dir(&root, &json);

        if depth >= 2 && !nodes.is_empty() {
            let targets: Vec<String> = nodes
                .iter()
                .take(MAX_CHILD_REQUESTS)
                .map(|n| n.path.clone())
                .collect();
            // Cap the burst: 25 simultaneous requests get the account throttled.
            let sem = Arc::new(Semaphore::new(MAX_PARALLEL_READS));
            let mut set = tokio::task::JoinSet::new();
            for target in targets {
                let token = self.token.clone();
                let sem = sem.clone();
                set.spawn(async move {
                    let _permit = sem.acquire_owned().await;
                    let res =
                        call_raw_read(&token, "GetIVR2Dir", params_of(&[("path", &target)])).await;
                    (target, res)
                });
            }
            let mut extra = Vec::new();
            while let Some(joined) = set.join_next().await {
                if let Ok((parent, Ok(child_json))) = joined {
                    self.seed_cache_from_dir(&parent, &child_json).await;
                    extra.extend(children_from_dir(&parent, &child_json));
                }
            }
            nodes.extend(extra);
        }

        nodes.sort_by_key(|n| natural_key(&n.path));
        nodes.dedup_by(|a, b| a.path == b.path);
        Ok(nodes)
    }

    /// List the files (not the sub-extensions) of one extension folder.
    pub async fn list_files(&self, path: &str) -> Result<Vec<FileEntry>, YemotError> {
        let canon = canon_ext(path)?;
        let json = self
            .call_read("GetIVR2Dir", params_of(&[("path", &canon)]))
            .await?;
        self.seed_cache_from_dir(&canon, &json).await;
        Ok(files_from_dir(&json))
    }

    /// Read any text file (ini / txt / …). Audio and binary files are refused.
    pub async fn get_text_file(&self, path: &str) -> Result<TextFile, YemotError> {
        let canon = canon_file(path)?;
        let ext = file_extension(&canon);
        if BINARY_EXTS.contains(&ext.as_str()) {
            return Err(YemotError::BadRequest(format!(
                "לא ניתן לקרוא קובץ מסוג {} כטקסט",
                ext
            )));
        }
        match self
            .call_read("GetTextFile", params_of(&[("what", &canon)]))
            .await
        {
            Ok(json) => {
                let meta = json
                    .get("file")
                    .and_then(|f| f.as_array().and_then(|a| a.first()).or(Some(f)))
                    .cloned()
                    .unwrap_or(Value::Null);
                let exists = meta.get("exists").and_then(|v| v.as_bool()).unwrap_or(true);
                Ok(TextFile {
                    exists,
                    contents: json
                        .get("contents")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                })
            }
            Err(e) if is_missing_file(&e) => Ok(TextFile {
                exists: false,
                contents: String::new(),
            }),
            Err(e) => Err(e),
        }
    }

    /// `GetSession`, whitelisted to three harmless fields. The response also
    /// carries `accessPassword` / `recordPassword`; those must never leave Rust.
    pub async fn get_system_info(&self) -> Result<SystemInfo, YemotError> {
        let json = self.call_read("GetSession", Map::new()).await?;
        Ok(SystemInfo {
            system: json
                .get("username")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            units: json.get("units").and_then(|v| v.as_f64()),
            units_expire: json
                .get("unitsExpireDate")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        })
    }

    /// Apply several `ext.ini` parameters in ONE `UpdateExtension` request,
    /// then re-read the file and report, per parameter, whether it stuck.
    pub async fn update_extension(
        &self,
        path: &str,
        params: &[(String, String)],
    ) -> Result<Vec<ParamOutcome>, YemotError> {
        let canon = canon_ext(path)?;
        if params.is_empty() {
            return Err(YemotError::BadRequest("לא נשלחו פרמטרים לעדכון".to_string()));
        }

        let mut body = Map::new();
        body.insert("path".to_string(), json!(canon));
        for (key, value) in params {
            let key = key.trim();
            if key.is_empty() {
                return Err(YemotError::BadRequest("שם פרמטר ריק".to_string()));
            }
            if key.eq_ignore_ascii_case("token") || key.eq_ignore_ascii_case("path") {
                return Err(YemotError::BadRequest(format!(
                    "שם פרמטר שמור ואינו מותר: {}",
                    key
                )));
            }
            if key.contains(['\r', '\n', '=']) || value.contains(['\r', '\n']) {
                return Err(YemotError::BadRequest(format!(
                    "ערך רב-שורתי אינו נתמך בעדכון שלוחה: {}",
                    key
                )));
            }
            if body.insert(key.to_string(), json!(value)).is_some() {
                return Err(YemotError::BadRequest(format!(
                    "הפרמטר {} נשלח יותר מפעם אחת",
                    key
                )));
            }
        }

        self.call("UpdateExtension", body).await?;
        self.cache.write().await.invalidate(&canon);

        // Verify by reading the file back — the API reports nothing per-param.
        let read = self.get_ext_ini(&canon).await;
        Ok(params
            .iter()
            .map(|(key, value)| match &read {
                Ok(r) => {
                    let current = r.ini.get(key.trim());
                    let applied = param_applied(current, value);
                    ParamOutcome {
                        key: key.trim().to_string(),
                        value: value.clone(),
                        applied,
                        note: if applied {
                            None
                        } else {
                            Some(format!(
                                "הערך בקובץ כעת: {}",
                                current.unwrap_or("(לא קיים)")
                            ))
                        },
                    }
                }
                Err(_) => ParamOutcome {
                    key: key.trim().to_string(),
                    value: value.clone(),
                    applied: false,
                    note: Some(NOTE_UNVERIFIED.to_string()),
                },
            })
            .collect())
    }

    /// Overwrite a text file completely. Returns the previous contents (for an
    /// undo), or `None` if the file did not exist.
    pub async fn upload_text_file(
        &self,
        path: &str,
        contents: &str,
    ) -> Result<Option<String>, YemotError> {
        let canon = canon_file(path)?;
        let ext = file_extension(&canon);
        if BINARY_EXTS.contains(&ext.as_str()) {
            return Err(YemotError::BadRequest(format!(
                "לא ניתן להעלות טקסט לקובץ מסוג {}",
                ext
            )));
        }

        let previous = match self.get_text_file(&canon).await {
            Ok(t) if t.exists => Some(t.contents),
            Ok(_) => None,
            Err(e) if is_missing_file(&e) => None,
            Err(e) => return Err(e),
        };

        let mut body = Map::new();
        body.insert("what".to_string(), json!(canon));
        body.insert("contents".to_string(), json!(contents));
        self.call("UploadTextFile", body).await?;

        self.cache.write().await.invalidate(&parent_of_file(&canon));
        Ok(previous)
    }
}

// ---------------------------------------------------------------------------
// Model-facing renderers (flat text, no JSON)
// ---------------------------------------------------------------------------

pub fn render_ext_read(path: &str, read: &ExtRead, max_bytes: usize) -> String {
    let mut out = format!(
        "ext={} exists={} size={} mtime={}\n",
        display_path(path),
        read.exists,
        read.size
            .map(|s| s.to_string())
            .unwrap_or_else(|| "-".to_string()),
        read.mtime.clone().unwrap_or_else(|| "-".to_string())
    );
    if !read.exists {
        out.push_str("(אין קובץ ext.ini — השלוחה אינה מוגדרת)\n");
        return out;
    }
    let (body, truncated) = read.ini.to_compact(max_bytes);
    let shown = body.lines().count();
    let total = read.ini.key_count();
    out.push_str(&body);
    if truncated {
        out.push_str(&format!("... [TRUNCATED: {} of {} keys]\n", shown, total));
    } else {
        out.push_str(&format!("({} keys)\n", total));
    }
    out
}

pub fn render_tree(nodes: &[ExtNode]) -> String {
    if nodes.is_empty() {
        return "(אין שלוחות)\n".to_string();
    }
    let mut out = String::new();
    for n in nodes.iter().take(MAX_TREE_LINES) {
        out.push_str(&format!(
            "{}  type={}  title={}\n",
            display_path(&n.path),
            if n.ext_type.is_empty() { "-" } else { &n.ext_type },
            if n.title.is_empty() { "-" } else { &n.title }
        ));
    }
    if nodes.len() > MAX_TREE_LINES {
        out.push_str(&format!("… ועוד {}\n", nodes.len() - MAX_TREE_LINES));
    }
    out
}

pub fn render_files(path: &str, files: &[FileEntry]) -> String {
    if files.is_empty() {
        return format!("ext={} (אין קבצים)\n", display_path(path));
    }
    let mut out = format!("ext={} files={}\n", display_path(path), files.len());
    for f in files.iter().take(MAX_FILE_LINES) {
        out.push_str(&format!(
            "{}  size={}  mtime={}\n",
            f.name,
            f.size.map(|s| s.to_string()).unwrap_or_else(|| "-".to_string()),
            f.mtime.clone().unwrap_or_else(|| "-".to_string())
        ));
    }
    if files.len() > MAX_FILE_LINES {
        out.push_str(&format!("… ועוד {}\n", files.len() - MAX_FILE_LINES));
    }
    out
}

pub fn render_outcomes(path: &str, outcomes: &[ParamOutcome]) -> String {
    let applied = outcomes.iter().filter(|o| o.applied).count();
    let mut out = format!(
        "update {} applied={}/{}\n",
        display_path(path),
        applied,
        outcomes.len()
    );
    for o in outcomes {
        if o.applied {
            out.push_str(&format!("{}={} OK\n", o.key, o.value));
        } else {
            out.push_str(&format!(
                "{}={} FAILED — {}\n",
                o.key,
                o.value,
                o.note.clone().unwrap_or_else(|| "לא הוחל".to_string())
            ));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Frontend DTOs
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct YemotSessionResult {
    pub success: bool,
    pub message: String,
    pub mfa_required: bool,
    pub mfa_token: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ExtParam {
    pub key: String,
    pub value: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ExtensionUpdateResult {
    pub success: bool,
    pub path: String,
    pub params: Vec<ParamOutcome>,
    pub message: String,
    pub verified: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct YemotLoginResult {
    pub success: bool,
    pub message: String,
    pub token: Option<String>,
    pub mfa_required: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MfaMethod {
    pub id: String,
    pub label: String,
    pub send_types: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MfaMethodsResult {
    pub success: bool,
    pub message: String,
    pub methods: Vec<MfaMethod>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct YemotSimpleResult {
    pub success: bool,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// URL + JSON body of one API call. Split out so a test can assert that no
/// credential ever reaches the query string.
///
/// A token is *not* a body parameter: it travels in the `authorization` header,
/// exactly like [`post_json`] does for the agent's calls.
fn post_parts(endpoint: &str, params: &[(&str, &str)]) -> (String, Value) {
    let mut body = Map::new();
    for (k, v) in params {
        body.insert((*k).to_string(), json!(v));
    }
    (format!("{}{}", YEMOT_API_BASE, endpoint), Value::Object(body))
}

/// One API call as POST + JSON body. The Yemot API accepts POST with a JSON
/// body on every endpoint ("את כל הבקשות ניתן לשלוח בGET או בPOST"), and unlike
/// a query string a body is not written to proxy / server access logs.
async fn yemot_post(
    endpoint: &str,
    token: Option<&str>,
    params: &[(&str, &str)],
) -> Result<Value, String> {
    let (url, body) = post_parts(endpoint, params);
    let mut req = http()
        .post(&url)
        .header("Content-Type", "application/json");
    if let Some(t) = token {
        req = req.header("authorization", t);
    }
    let res = req
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("שגיאת רשת: {}", e.without_url()))?;
    res.json::<Value>()
        .await
        .map_err(|e| format!("שגיאת פענוח תשובה: {}", e.without_url()))
}

#[tauri::command]
pub async fn check_yemot_token(token: String) -> Result<YemotSessionResult, String> {
    if token.trim().is_empty() {
        return Ok(YemotSessionResult {
            success: false,
            message: "טוקן ימות המשיח ריק".to_string(),
            mfa_required: false,
            mfa_token: None,
        });
    }

    // We look only at responseStatus / message / mfaToken and never surface any
    // other GetSession field (it carries accessPassword / recordPassword etc.).
    let json = match post_json(token.trim(), "GetSession", Map::new()).await {
        Ok(j) => j,
        Err(YemotError::Network(e)) => return Err(format!("שגיאת רשת: {}", e)),
        Err(e) => {
            return Ok(YemotSessionResult {
                success: false,
                message: render_error(&e),
                mfa_required: false,
                mfa_token: None,
            })
        }
    };

    // The MFA session handle is not a credential; the legacy MFA modal needs it.
    let mfa_token = json
        .get("mfaToken")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let err = match classify_response(&json) {
        Ok(()) => {
            return Ok(YemotSessionResult {
                success: true,
                message: "טוקן תקין ומחובר בהצלחה".to_string(),
                mfa_required: false,
                mfa_token: None,
            })
        }
        Err(YemotError::MfaRequired) => {
            return Ok(YemotSessionResult {
                success: false,
                message: "נדרש אימות דו-שלבי (MFA)".to_string(),
                mfa_required: true,
                mfa_token,
            })
        }
        Err(e) => e,
    };

    Ok(YemotSessionResult {
        success: false,
        message: match &err {
            YemotError::Api { message, .. }
            | YemotError::Forbidden(message)
            | YemotError::SessionExpired(message)
            | YemotError::BadRequest(message)
                if !message.is_empty() =>
            {
                message.clone()
            }
            _ => "טוקן לא תקין או שפג תוקפו".to_string(),
        },
        mfa_required: false,
        mfa_token: None,
    })
}

/// Apply several parameters to one extension in a single API request.
#[tauri::command]
pub async fn execute_yemot_actions(
    token: String,
    path: String,
    params: Vec<ExtParam>,
) -> Result<ExtensionUpdateResult, String> {
    let client = YemotClient::new(token);
    let pairs: Vec<(String, String)> = params.into_iter().map(|p| (p.key, p.value)).collect();
    let canon = canon_ext(&path).map(|c| display_path(&c)).unwrap_or(path);

    match client.update_extension(&canon, &pairs).await {
        Ok(outcomes) => {
            let verified = !outcomes
                .iter()
                .any(|o| o.note.as_deref() == Some(NOTE_UNVERIFIED));
            let applied = outcomes.iter().filter(|o| o.applied).count();
            Ok(ExtensionUpdateResult {
                success: if verified {
                    applied == outcomes.len()
                } else {
                    true
                },
                path: canon,
                message: if !verified {
                    "העדכון נשלח אך לא ניתן היה לאמת אותו".to_string()
                } else {
                    format!("הוחלו {} מתוך {} פרמטרים", applied, outcomes.len())
                },
                verified,
                params: outcomes,
            })
        }
        Err(e) => Ok(ExtensionUpdateResult {
            success: false,
            path: canon,
            params: Vec::new(),
            message: render_error(&e),
            verified: false,
        }),
    }
}

/// Login with system number + password. Returns a token; indicates whether
/// MFA (two-factor) verification is still required before using it.
#[tauri::command]
pub async fn login_yemot(username: String, password: String) -> Result<YemotLoginResult, String> {
    if username.trim().is_empty() || password.trim().is_empty() {
        return Ok(YemotLoginResult {
            success: false,
            message: "יש להזין מספר מערכת וסיסמה".to_string(),
            token: None,
            mfa_required: false,
        });
    }

    // POST: a password in a query string ends up in every access log on the way.
    let json = yemot_post(
        "Login",
        None,
        &[("username", username.trim()), ("password", &password)],
    )
    .await?;
    let msg = json["message"].as_str().unwrap_or("");

    if classify_response(&json).is_err() {
        return Ok(YemotLoginResult {
            success: false,
            message: if msg.is_empty() {
                "ההתחברות נכשלה".to_string()
            } else {
                msg.to_string()
            },
            token: None,
            mfa_required: false,
        });
    }

    let token = json["token"].as_str().unwrap_or("").to_string();
    if token.is_empty() {
        return Ok(YemotLoginResult {
            success: false,
            message: "טוקן לא התקבל מהמערכת".to_string(),
            token: None,
            mfa_required: false,
        });
    }

    // Check global MFA status for this session. A failure here must NOT throw
    // the token away: the login itself succeeded, and the caller can still use
    // the token (the MFA screen is the safe assumption when the probe failed).
    let is_pass = match yemot_post("MFASession", Some(&token), &[("action", "isPass")]).await {
        Ok(mfa_json) => mfa_json["isPass"].as_bool().unwrap_or(false),
        Err(e) => {
            eprintln!("login_yemot: MFASession probe failed: {}", e);
            false
        }
    };

    Ok(YemotLoginResult {
        success: true,
        message: if is_pass {
            "התחברות הושלמה בהצלחה".to_string()
        } else {
            "נדרש אימות דו-שלבי (MFA)".to_string()
        },
        token: Some(token),
        mfa_required: !is_pass,
    })
}

/// Get available MFA verification methods (call / SMS etc.) for a session.
#[tauri::command]
pub async fn get_mfa_methods(token: String) -> Result<MfaMethodsResult, String> {
    let json = yemot_post(
        "MFASession",
        Some(token.trim()),
        &[("action", "getMFAMethods")],
    )
    .await?;

    let mut methods = Vec::new();
    if let Some(arr) = json["mfaMethods"].as_array() {
        for m in arr {
            let id = m["ID"].as_i64().map(|v| v.to_string()).unwrap_or_default();
            let value = m["VALUE"].as_str().unwrap_or("");
            let nike = m["NIKE"].as_str().unwrap_or("");
            let label = if nike.is_empty() {
                value.to_string()
            } else {
                format!("{} ({})", value, nike)
            };
            let send_types: Vec<String> = m["SEND_TYPE"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            if !id.is_empty() {
                methods.push(MfaMethod {
                    id,
                    label,
                    send_types,
                });
            }
        }
    }

    if methods.is_empty() {
        Ok(MfaMethodsResult {
            success: false,
            message: "לא נמצאו שיטות אימות זמינות. יש להגדיר שיטות אימות במערכת".to_string(),
            methods,
        })
    } else {
        Ok(MfaMethodsResult {
            success: true,
            message: "OK".to_string(),
            methods,
        })
    }
}

/// Send an MFA verification code using the selected method & send type.
#[tauri::command]
pub async fn send_mfa_code(
    token: String,
    mfa_id: String,
    send_type: String,
) -> Result<YemotSimpleResult, String> {
    let json = yemot_post(
        "MFASession",
        Some(token.trim()),
        &[
            ("action", "sendMFA"),
            ("mfaId", &mfa_id),
            ("mfaSendType", &send_type),
            ("lang", "HE"),
        ],
    )
    .await?;
    let msg = json["message"].as_str().unwrap_or("");

    match classify_response(&json) {
        Ok(()) => Ok(YemotSimpleResult {
            success: true,
            message: "קוד אימות נשלח בהצלחה".to_string(),
        }),
        Err(_) => Ok(YemotSimpleResult {
            success: false,
            message: if msg.is_empty() {
                "שליחת הקוד נכשלה".to_string()
            } else {
                format!("שגיאה: {}", msg)
            },
        }),
    }
}

/// Validate the MFA code the user received.
#[tauri::command]
pub async fn validate_mfa_code(token: String, code: String) -> Result<YemotSimpleResult, String> {
    let json = yemot_post(
        "MFASession",
        Some(token.trim()),
        &[
            ("action", "validMFA"),
            ("mfaCode", &code),
            ("mfaRememberMe", "false"),
        ],
    )
    .await?;
    let valid_status = json["mfa_valid_status"].as_str().unwrap_or("");

    if valid_status == "VALID" {
        Ok(YemotSimpleResult {
            success: true,
            message: "האימות הושלם בהצלחה!".to_string(),
        })
    } else if valid_status == "OVERTRY" {
        Ok(YemotSimpleResult {
            success: false,
            message: "חרגת ממספר הניסיונות. יש לשלוח קוד חדש".to_string(),
        })
    } else {
        let left = json["mfa_valid_left"].as_i64();
        let message = match left {
            Some(n) => format!("קוד שגוי. נותרו {} ניסיונות", n),
            None => "הקוד שהוזן שגוי".to_string(),
        };
        Ok(YemotSimpleResult {
            success: false,
            message,
        })
    }
}

/// Logout (invalidate the token) — performed locally, never via the script.
#[tauri::command]
pub async fn logout_yemot(token: String) -> Result<YemotSimpleResult, String> {
    // Undo records hold an authenticated client each; none of them may outlive
    // the session the user just ended.
    crate::agent::runner::clear_write_state().await;
    let json = yemot_post("Logout", Some(token.trim()), &[]).await?;
    match classify_response(&json) {
        Ok(()) => Ok(YemotSimpleResult {
            success: true,
            message: "התנתקות בוצעה בהצלחה".to_string(),
        }),
        Err(e) => Ok(YemotSimpleResult {
            success: false,
            message: format!("התנתקות נכשלה: {}", render_error(&e)),
        }),
    }
}

// ---------------------------------------------------------------------------
// Tests (no network)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canon_ext_accepts_every_documented_form() {
        for input in ["/1/2", "1/2", "ivr2:/1/2", "ivr2:1/2", "ivr/1/2", "1/2/"] {
            assert_eq!(canon_ext(input).unwrap(), "ivr2:/1/2", "input: {}", input);
        }
    }

    #[test]
    fn canon_ext_maps_root_forms() {
        for input in ["", "/", "ivr2:/", "ivr2:"] {
            assert_eq!(canon_ext(input).unwrap(), "ivr2:/", "input: {}", input);
        }
    }

    #[test]
    fn canon_ext_rejects_non_numeric_and_empty_segments() {
        for input in ["/1/a", "../1", "1//2", "ivr2:/1/ext.ini", "1 2"] {
            assert!(canon_ext(input).is_err(), "should reject: {}", input);
        }
    }

    #[test]
    fn canon_file_builds_full_paths() {
        assert_eq!(canon_file("1/2/ext.ini").unwrap(), "ivr2:/1/2/ext.ini");
        assert_eq!(canon_file("ivr2:/4/ext.ini").unwrap(), "ivr2:/4/ext.ini");
        assert_eq!(canon_file("ivr/4/ext.ini").unwrap(), "ivr2:/4/ext.ini");
        assert_eq!(canon_file("ext.ini").unwrap(), "ivr2:/ext.ini");
        assert!(canon_file("1/2").is_err());
        assert!(canon_file("1/../x.ini").is_err());
    }

    #[test]
    fn display_and_ini_path_helpers() {
        assert_eq!(display_path("ivr2:/1/2"), "/1/2");
        assert_eq!(display_path("ivr2:/"), "/");
        assert_eq!(ext_ini_path("ivr2:/"), "ivr2:/ext.ini");
        assert_eq!(ext_ini_path("ivr2:/1"), "ivr2:/1/ext.ini");
        assert_eq!(parent_of_file("ivr2:/1/2/ext.ini"), "ivr2:/1/2");
        assert_eq!(parent_of_file("ivr2:/ext.ini"), "ivr2:/");
    }

    #[test]
    fn classifies_ok() {
        assert!(classify_response(&json!({"responseStatus": "OK"})).is_ok());
    }

    #[test]
    fn classifies_mfa_required_from_error_and_forbidden() {
        let as_error = json!({"responseStatus": "ERROR", "message": "MFA_REQUIRED"});
        let as_forbidden = json!({"responseStatus": "FORBIDDEN", "message": "MFA_REQUIRED"});
        assert_eq!(
            classify_response(&as_error).unwrap_err(),
            YemotError::MfaRequired
        );
        assert_eq!(
            classify_response(&as_forbidden).unwrap_err(),
            YemotError::MfaRequired
        );
    }

    #[test]
    fn classifies_plain_error_with_code() {
        let v = json!({
            "yemotAPIVersion": "1",
            "responseStatus": "ERROR",
            "message": "Username or password is incorrect",
            "messageCode": 1
        });
        // "incorrect" + no token/session word => plain API error.
        assert_eq!(
            classify_response(&v).unwrap_err(),
            YemotError::Api {
                code: Some(1),
                message: "Username or password is incorrect".to_string()
            }
        );
    }

    #[test]
    fn classifies_dead_session_and_forbidden_and_missing_status() {
        assert!(matches!(
            classify_response(&json!({"responseStatus": "ERROR", "message": "token is invalid"}))
                .unwrap_err(),
            YemotError::SessionExpired(_)
        ));
        assert!(matches!(
            classify_response(&json!({"responseStatus": "FORBIDDEN", "message": "ip blocked"}))
                .unwrap_err(),
            YemotError::Forbidden(_)
        ));
        assert!(matches!(
            classify_response(&json!({"foo": 1})).unwrap_err(),
            YemotError::BadRequest(_)
        ));
        assert!(matches!(
            classify_response(&json!({"responseStatus": "EXCEPTION", "message": "boom"}))
                .unwrap_err(),
            YemotError::Api { .. }
        ));
    }

    #[test]
    fn missing_file_detection() {
        assert!(is_missing_file(&YemotError::Api {
            code: None,
            message: "file does not exist".to_string()
        }));
        assert!(!is_missing_file(&YemotError::Api {
            code: None,
            message: "some other problem".to_string()
        }));
    }

    #[test]
    fn render_error_codes() {
        assert!(render_error(&YemotError::MfaRequired).starts_with("SESSION_EXPIRED: "));
        assert!(render_error(&YemotError::MfaRequired).ends_with("Stop and wait — do not retry."));
        assert!(render_error(&YemotError::SessionExpired("gone".into()))
            .starts_with("SESSION_EXPIRED: gone"));
        assert_eq!(
            render_error(&YemotError::Forbidden("nope".into())),
            "FORBIDDEN: nope"
        );
        assert_eq!(
            render_error(&YemotError::BadRequest("bad path".into())),
            "BAD_REQUEST: bad path"
        );
        assert_eq!(
            render_error(&YemotError::Api {
                code: Some(7),
                message: "oops".into()
            }),
            "ERROR[7]: oops"
        );
        assert_eq!(
            render_error(&YemotError::Api {
                code: None,
                message: "oops".into()
            }),
            "ERROR[-]: oops"
        );
        let net = render_error(&YemotError::Network("timeout".into()));
        assert!(net.starts_with("NETWORK_ERROR: timeout"));
        assert!(net.contains("read the extension config to verify before retrying."));
    }

    #[test]
    fn renders_ext_read() {
        let read = ExtRead {
            exists: true,
            ini: ExtIni::parse("; c\ntype=menu\ntitle=בדיקה\n"),
            size: Some(214),
            mtime: Some("01/01/2026 10:00".to_string()),
            source: ExtSource::File,
        };
        let out = render_ext_read("ivr2:/1/2", &read, DEFAULT_MAX_BYTES);
        assert_eq!(
            out,
            "ext=/1/2 exists=true size=214 mtime=01/01/2026 10:00\n\
             type=menu\ntitle=בדיקה\n(2 keys)\n"
        );
    }

    #[test]
    fn renders_ext_read_truncated_and_missing() {
        let read = ExtRead {
            exists: true,
            ini: ExtIni::parse("a=1\nb=2\nc=3"),
            size: None,
            mtime: None,
            source: ExtSource::File,
        };
        let out = render_ext_read("ivr2:/1", &read, 8);
        assert!(out.contains("size=- mtime=-"));
        assert!(out.ends_with("... [TRUNCATED: 2 of 3 keys]\n"));

        let missing = ExtRead {
            exists: false,
            ini: ExtIni::default(),
            size: None,
            mtime: None,
            source: ExtSource::File,
        };
        let out = render_ext_read("ivr2:/9", &missing, DEFAULT_MAX_BYTES);
        assert!(out.starts_with("ext=/9 exists=false"));
        assert!(out.contains("אין קובץ ext.ini"));
    }

    #[test]
    fn renders_tree_with_cap() {
        let nodes = vec![
            ExtNode {
                path: "ivr2:/1".into(),
                ext_type: "menu".into(),
                title: "ראשי".into(),
            },
            ExtNode {
                path: "ivr2:/2".into(),
                ext_type: String::new(),
                title: String::new(),
            },
        ];
        let out = render_tree(&nodes);
        assert_eq!(out, "/1  type=menu  title=ראשי\n/2  type=-  title=-\n");

        let many: Vec<ExtNode> = (0..65)
            .map(|i| ExtNode {
                path: format!("ivr2:/{}", i),
                ext_type: "menu".into(),
                title: "t".into(),
            })
            .collect();
        let out = render_tree(&many);
        assert_eq!(out.lines().count(), MAX_TREE_LINES + 1);
        assert!(out.ends_with("… ועוד 5\n"));
        assert_eq!(render_tree(&[]), "(אין שלוחות)\n");
    }

    #[test]
    fn renders_outcomes() {
        let outcomes = vec![
            ParamOutcome {
                key: "type".into(),
                value: "menu".into(),
                applied: true,
                note: None,
            },
            ParamOutcome {
                key: "title".into(),
                value: "x".into(),
                applied: false,
                note: Some("הערך בקובץ כעת: y".into()),
            },
        ];
        let out = render_outcomes("ivr2:/1/2", &outcomes);
        assert_eq!(
            out,
            "update /1/2 applied=1/2\ntype=menu OK\ntitle=x FAILED — הערך בקובץ כעת: y\n"
        );
    }

    #[test]
    fn ini_from_object_seeds_pairs() {
        let obj = json!({"type": "menu", "num": 5});
        let ini = ini_from_object(obj.as_object().unwrap());
        assert_eq!(ini.get("type"), Some("menu"));
        assert_eq!(ini.get("num"), Some("5"));
    }

    #[test]
    fn children_from_dir_extracts_extensions() {
        let v = json!({
            "dirs": [
                {"name": "1", "extType": "menu", "extTitle": "ראשי"},
                {"name": "2"},
                {"name": "weird-name", "extType": "menu"}
            ]
        });
        let nodes = children_from_dir("ivr2:/", &v);
        assert_eq!(nodes.len(), 2); // "weird-name" is rejected by canon_ext
        assert_eq!(nodes[0].path, "ivr2:/1");
        assert_eq!(nodes[0].title, "ראשי");
        assert_eq!(nodes[1].ext_type, "");

        let nested = children_from_dir("ivr2:/3", &v);
        assert_eq!(nested[0].path, "ivr2:/3/1");
    }

    #[test]
    fn client_debug_redacts_the_token() {
        let c = YemotClient::new("077000000:1234");
        let dbg = format!("{:?}", c);
        assert!(dbg.contains("<redacted>"));
        assert!(!dbg.contains("1234"));
    }

    #[tokio::test]
    async fn cache_helpers_work_without_network() {
        let c = YemotClient::new("t");
        assert!(!c.has_ext("/1").await);
        c.cache_put(
            "ivr2:/1",
            &ExtRead {
                exists: true,
                ini: ExtIni::parse("type=menu"),
                size: None,
                mtime: None,
                source: ExtSource::File,
            },
        )
        .await;
        assert!(c.has_ext("1").await);
        assert!(c.has_ext("ivr2:/1").await);
        c.clear().await;
        assert!(!c.has_ext("/1").await);
        // A bad path is never "cached".
        assert!(!c.has_ext("/a").await);
    }

    #[tokio::test]
    async fn update_extension_rejects_bad_params_before_any_request() {
        let c = YemotClient::new("t");
        assert!(matches!(
            c.update_extension("/1", &[]).await.unwrap_err(),
            YemotError::BadRequest(_)
        ));
        assert!(matches!(
            c.update_extension("/1", &[("title".into(), "a\nb".into())])
                .await
                .unwrap_err(),
            YemotError::BadRequest(_)
        ));
        assert!(matches!(
            c.update_extension("/1", &[("path".into(), "x".into())])
                .await
                .unwrap_err(),
            YemotError::BadRequest(_)
        ));
        assert!(matches!(
            c.update_extension(
                "/1",
                &[("type".into(), "menu".into()), ("type".into(), "api".into())]
            )
            .await
            .unwrap_err(),
            YemotError::BadRequest(_)
        ));
        assert!(matches!(
            c.update_extension("/a", &[("type".into(), "menu".into())])
                .await
                .unwrap_err(),
            YemotError::BadRequest(_)
        ));
    }

    // -- credentials never travel in a URL ---------------------------------

    #[test]
    fn credentials_go_in_the_body_never_in_the_url() {
        let (url, body) = post_parts("Login", &[("username", "077000000"), ("password", "s3cr3t!")]);
        assert_eq!(url, "https://www.call2all.co.il/ym/api/Login");
        assert!(!url.contains('?'), "no query string at all: {}", url);
        assert!(!url.contains("s3cr3t"), "password leaked into the URL: {}", url);
        assert_eq!(body["password"], json!("s3cr3t!"));
        assert_eq!(body["username"], json!("077000000"));
        // the token is a header, never a body/query parameter
        let (url, body) = post_parts("MFASession", &[("action", "isPass")]);
        assert!(!url.contains("token"));
        assert!(body.get("token").is_none());
    }

    // -- write verification -------------------------------------------------

    #[test]
    fn empty_value_counts_as_applied_when_the_key_is_gone_or_blank() {
        assert!(param_applied(None, ""));
        assert!(param_applied(Some(""), ""));
        assert!(param_applied(Some("   "), " "));
        assert!(!param_applied(None, "menu"));
        assert!(!param_applied(Some("old"), ""));
    }

    #[test]
    fn verification_trims_both_sides() {
        assert!(param_applied(Some("menu"), " menu "));
        assert!(param_applied(Some(" menu "), "menu"));
        assert!(!param_applied(Some("menu2"), "menu"));
    }

    // -- file names ---------------------------------------------------------

    #[test]
    fn filenames_allow_hebrew_and_reject_the_dangerous_set() {
        assert!(is_valid_filename("רשימת חברים.txt"));
        assert!(is_valid_filename("000.wav"));
        assert!(is_valid_filename("a-b_c.2.ini"));
        for bad in [
            "", "ext.ini/", "a/b.txt", "a\\b.txt", "a:b.txt", "a*.txt", "a?.txt",
            "a\"b.txt", "a<b.txt", "a>b.txt", "a|b.txt", "..txt", "no-extension",
            ".hidden.txt", "a..b.txt",
        ] {
            assert!(!is_valid_filename(bad), "should reject: {:?}", bad);
        }
        assert!(!is_valid_filename("a\u{7}b.txt"));
        assert_eq!(canon_file("/1/רשימה.txt").unwrap(), "ivr2:/1/רשימה.txt");
    }

    // -- cache source -------------------------------------------------------

    #[tokio::test]
    async fn listing_seeds_are_marked_and_never_overwrite_a_file_read() {
        let c = YemotClient::new("t");
        let dir = json!({"extIni": {"type": "menu"}});
        c.seed_cache_from_dir("ivr2:/1", &dir).await;
        let read = c.get_ext_ini("/1").await.unwrap();
        assert_eq!(read.source, ExtSource::Listing);
        assert_eq!(read.ini.get("type"), Some("menu"));

        // a real file read wins, and a later listing must not downgrade it
        c.cache_put(
            "ivr2:/1",
            &ExtRead {
                exists: true,
                ini: ExtIni::parse("type=menu\ntitle=x"),
                size: Some(20),
                mtime: None,
                source: ExtSource::File,
            },
        )
        .await;
        c.seed_cache_from_dir("ivr2:/1", &dir).await;
        let read = c.get_ext_ini("/1").await.unwrap();
        assert_eq!(read.source, ExtSource::File);
        assert_eq!(read.ini.get("title"), Some("x"));
    }

    #[test]
    fn snapshot_hash_tracks_content_and_existence() {
        let mk = |exists, text: &str| ExtRead {
            exists,
            ini: ExtIni::parse(text),
            size: None,
            mtime: None,
            source: ExtSource::File,
        };
        assert_eq!(mk(true, "type=menu").snapshot_hash(), mk(true, "type=menu").snapshot_hash());
        assert_ne!(mk(true, "type=menu").snapshot_hash(), mk(true, "type=api").snapshot_hash());
        assert_ne!(mk(true, "").snapshot_hash(), mk(false, "").snapshot_hash());
    }

    // -- directory files ----------------------------------------------------

    #[test]
    fn files_from_dir_collects_every_bucket_once() {
        let v = json!({
            "dirs": [{"name": "1"}],
            "files": [{"name": "000.wav", "size": 12, "mtime": "01/01/2026 10:00"}],
            "ini": [{"name": "ext.ini", "size": 40}],
            "messages": [{"name": "M0000.wav"}],
            "html": [{"name": "ext.ini"}]
        });
        let files = files_from_dir(&v);
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["000.wav", "M0000.wav", "ext.ini"]);
        assert_eq!(files[0].size, Some(12));
        assert_eq!(files[1].mtime, None);
        assert!(files_from_dir(&json!({"dirs": []})).is_empty());
    }

    #[test]
    fn renders_files() {
        let files = vec![
            FileEntry { name: "000.wav".into(), size: Some(12), mtime: Some("01/01/2026".into()) },
            FileEntry { name: "ext.ini".into(), size: None, mtime: None },
        ];
        assert_eq!(
            render_files("ivr2:/1", &files),
            "ext=/1 files=2\n000.wav  size=12  mtime=01/01/2026\next.ini  size=-  mtime=-\n"
        );
        assert_eq!(render_files("ivr2:/1", &[]), "ext=/1 (אין קבצים)\n");
    }

    #[tokio::test]
    async fn get_text_file_rejects_binary_before_any_request() {
        let c = YemotClient::new("t");
        assert!(matches!(
            c.get_text_file("/1/000.wav").await.unwrap_err(),
            YemotError::BadRequest(_)
        ));
        assert!(matches!(
            c.upload_text_file("/1/000.mp3", "x").await.unwrap_err(),
            YemotError::BadRequest(_)
        ));
    }
}
