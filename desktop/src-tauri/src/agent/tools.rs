//! Tool specs + dispatcher.
//!
//! Two rules shape everything here:
//!   * results are flat `key=value` text (the yemot renderers), never JSON —
//!     JSON costs ~40% more tokens for the same content and models read the
//!     flat form more reliably;
//!   * a write never happens as a side effect of the model talking. In deferred
//!     mode `set_extension_params` returns a synthetic, byte-identical receipt
//!     and the real call waits for `approve_actions`.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use tauri::AppHandle;
use tokio::sync::Mutex;

use crate::knowledge::{self, SearchOpts};
use crate::yemot::{self, ExtRead, ExtSource, YemotClient, YemotError};

use super::events::{self, ActionParam, DiffRowDto, ProposedAction};
use super::providers::openai::INVALID_ARGUMENTS_KEY;
use super::runner::Attachment;
use super::types::{ToolKind, ToolSpec};

/// Backstop cap for a single tool result. Per-tool caps ([`cap_for`]) are much
/// tighter — this one only catches a tool that has no entry of its own.
pub const MAX_RESULT_TOKENS: usize = 3_000;

/// How many tokens a result may cost, per tool. A knowledge search is a pointer
/// list, an extension read is a dozen lines — paying 3k for either is waste that
/// repeats on every later turn, because tool results stay in the prompt.
pub fn cap_for(tool: &str) -> usize {
    match tool {
        "search_knowledge" => 1_200,
        "get_knowledge_section" => 2_500,
        "lookup_param" => 1_500,
        "get_extension_config" => 800,
        "list_extensions" => 1_200,
        "list_files" => 800,
        "get_text_file" => 1_500,
        "get_system_info" => 300,
        _ => MAX_RESULT_TOKENS,
    }
}

/// The byte-identical receipt a deferred write returns. Byte stability matters:
/// this text repeats once per write and a varying one would break the cache.
pub const PENDING_NOTE: &str = "נרשמה לאישור, טרם בוצעה. המשך לשלוחה הבאה.";

/// Yemot capabilities that must never be reachable from a prompt, even if a
/// future refactor adds them to the client.
pub const DENIED_TOOLS: &[&str] = &[
    "file_action",
    "delete_extension",
    "run_tzintuk",
    "run_campaign",
    "schedule_campaign",
    "send_sms",
    "send_fax",
    "transfer_units",
    "set_password",
    "set_customer_details",
    "kill_session",
    "call_action",
];

// ---------------------------------------------------------------------------
// Specs
// ---------------------------------------------------------------------------

fn obj(props: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": props,
        "required": required,
        "additionalProperties": false
    })
}

/// The tool list, in a fixed order. Built from consts only — the serialized
/// `tools` array must be byte-identical on every turn of every run.
pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "search_knowledge",
            description: "חפש במאגר הידע של ימות המשיח לפי תיאור חופשי של המשימה. השתמש בזה כשאתה לא יודע איזה מודול או אילו הגדרות נדרשות. מחזיר קטעים מתוך מסמכי הידע עם שם הקובץ והכותרת, להעמקה בהם עם get_knowledge_section.",
            input_schema: obj(json!({ "query": { "type": "string" } }), &["query"]),
            kind: ToolKind::ReadOnly,
        },
        ToolSpec {
            name: "get_knowledge_section",
            description: "קרא סעיף שלם ממסמך ידע לפי שם קובץ וכותרת, אחרי ש-search_knowledge הצביע עליו. בלי heading מוחזרים הפתיח ותוכן העניינים של הקובץ. זה המקור המוסמך לשמות ההגדרות - אל תכתוב מפתח שלא הופיע כאן.",
            input_schema: obj(
                json!({
                    "file": { "type": "string" },
                    "heading": { "type": ["string", "null"] }
                }),
                &["file", "heading"],
            ),
            kind: ToolKind::ReadOnly,
        },
        ToolSpec {
            name: "lookup_param",
            description: "בדוק הגדרה בודדת (למשל type או enter_idd) ומצא את כל מופעיה המתועדים בהקשר. השתמש בזה כדי לאמת שם מפתח לפני כתיבה, או כדי להבין מה ערך חוקי. אם ההגדרה לא נמצאה - היא אינה מתועדת ואין לכתוב אותה.",
            input_schema: obj(json!({ "key": { "type": "string" } }), &["key"]),
            kind: ToolKind::ReadOnly,
        },
        ToolSpec {
            name: "get_extension_config",
            description: "קרא את קובץ ext.ini של שלוחה קיימת בנתיב כמו /3 או /1/2. מחזיר את ההגדרות הנוכחיות, או exists=false אם השלוחה אינה מוגדרת. חובה לקרוא שלוחה בכלי הזה לפני כל כתיבה אליה.",
            input_schema: obj(json!({ "path": { "type": "string" } }), &["path"]),
            kind: ToolKind::ReadOnly,
        },
        ToolSpec {
            name: "list_extensions",
            description: "הצג את השלוחות הקיימות תחת נתיב, עם הסוג והכותרת של כל אחת. depth=1 מחזיר את הבנות הישירות, depth=2 גם את הנכדות. שימושי כדי לבחור מספר שלוחה פנוי או להבין את מבנה המרכזייה.",
            input_schema: obj(
                json!({
                    "path": { "type": "string" },
                    "depth": { "type": "integer" }
                }),
                &["path", "depth"],
            ),
            kind: ToolKind::ReadOnly,
        },
        ToolSpec {
            name: "list_files",
            description: "הצג את הקבצים הקיימים בשלוחה לפי נתיב כמו /3, עם השם, הגודל בבתים ותאריך השינוי האחרון. כולל קבצי שמע, הודעות מערכת וקבצי ini. שימושי כדי לבדוק אם הקלטה כבר הועלתה לשלוחה, או איזה קובץ רשימה קיים בה.",
            input_schema: obj(json!({ "path": { "type": "string" } }), &["path"]),
            kind: ToolKind::ReadOnly,
        },
        ToolSpec {
            name: "get_text_file",
            description: "קרא קובץ טקסט כלשהו מהמערכת לפי נתיב מלא, למשל ivr2:/3/ext.ini או קובץ רשימה. מיועד לקבצים שאינם ext.ini של שלוחה. קבצי שמע וקבצים בינאריים אינם ניתנים לקריאה.",
            input_schema: obj(json!({ "path": { "type": "string" } }), &["path"]),
            kind: ToolKind::ReadOnly,
        },
        ToolSpec {
            name: "get_system_info",
            description: "פרטי המערכת של המשתמש: מספר המערכת, יתרת יחידות ותאריך תפוגה. השתמש בזה רק כשהמשתמש שאל על היתרה או כשנדרש מספר המערכת לצורך הגדרה.",
            input_schema: obj(json!({}), &[]),
            kind: ToolKind::ReadOnly,
        },
        ToolSpec {
            name: "set_extension_params",
            description: "כתוב הגדרות לקובץ ext.ini של שלוחה. הכתיבה ממזגת לתוך הקובץ הקיים, לכן שלח רק מפתחות שמשתנים, ובשלוחה חדשה תמיד גם type. חובה לקרוא את השלוחה עם get_extension_config לפני כן. הכתיבה נרשמת לאישור המשתמש ואינה מתבצעת מיד.",
            input_schema: obj(
                json!({
                    "path": { "type": "string" },
                    "params": {
                        "type": "array",
                        "items": obj(
                            json!({ "key": { "type": "string" }, "value": { "type": "string" } }),
                            &["key", "value"]
                        )
                    },
                    "reason": { "type": "string" }
                }),
                &["path", "params", "reason"],
            ),
            kind: ToolKind::Mutating,
        },
        ToolSpec {
            name: "upload_text_file",
            description: "החלף לחלוטין תוכן של קובץ טקסט בנתיב מלא. הקובץ הקודם נדרס, לכן השתמש בזה רק לקבצי רשימות או תוכן שנוצר מחדש, ולא לעריכת ext.ini של שלוחה. קובץ בשם NNN.tts, למשל 000.tts, הוא קובץ טקסט שהמערכת מקריאה למתקשר בהקראה ממוחשבת (text to speech) במקום הקלטה. גם כאן הפעולה נרשמת לאישור המשתמש ואינה מתבצעת מיד.",
            input_schema: obj(
                json!({
                    "path": { "type": "string" },
                    "contents": { "type": "string" },
                    "reason": { "type": "string" }
                }),
                &["path", "contents", "reason"],
            ),
            kind: ToolKind::Mutating,
        },
        ToolSpec {
            name: "upload_audio_file",
            description: "העלה קובץ שמע שהמשתמש צירף למשימה אל שלוחה, לפי המזהה שמופיע ברשימת הקבצים המצורפים. נתיב היעד חייב לכלול שם קובץ עם סיומת wav, למשל /3/000.wav, והמערכת ממירה את הקובץ המקורי בעצמה. אין באפשרותך ליצור קבצי שמע חדשים, רק להעלות קובץ שצורף. גם כאן הפעולה נרשמת לאישור המשתמש ואינה מתבצעת מיד.",
            input_schema: obj(
                json!({
                    "dest_path": { "type": "string" },
                    "attachment_id": { "type": "string" },
                    "reason": { "type": "string" }
                }),
                &["dest_path", "attachment_id", "reason"],
            ),
            kind: ToolKind::Mutating,
        },
    ]
}

pub fn spec_of(name: &str) -> Option<ToolSpec> {
    tool_specs().into_iter().find(|t| t.name == name)
}

pub fn is_mutating(name: &str) -> bool {
    spec_of(name).map(|t| t.kind == ToolKind::Mutating).unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Context + result
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ToolOutcome {
    pub content: String,
    pub is_error: bool,
}

impl ToolOutcome {
    fn ok(content: impl Into<String>) -> Self {
        ToolOutcome { content: content.into(), is_error: false }
    }
    fn err(content: impl Into<String>) -> Self {
        ToolOutcome { content: content.into(), is_error: true }
    }

    /// ≤120 chars, first line only — the `summary` of `agent:tool_finished`.
    pub fn summary(&self) -> String {
        let line = self.content.lines().next().unwrap_or("").trim();
        if line.chars().count() > 120 {
            line.chars().take(117).collect::<String>() + "…"
        } else {
            line.to_string()
        }
    }
}

pub struct ToolCtx {
    pub app: AppHandle,
    pub run_id: String,
    pub client: Arc<YemotClient>,
    pub auto_apply: bool,
    /// Canonical paths already read *from the file itself* in this run — the
    /// read-before-write gate. A directory-listing snapshot does not count.
    pub reads: Arc<Mutex<HashSet<String>>>,
    /// The `ext.ini` each of those reads returned, so a write can build its diff
    /// without paying for a second read.
    pub ext_reads: Arc<Mutex<HashMap<String, ExtRead>>>,
    pub proposed: Arc<Mutex<Vec<ProposedAction>>>,
    pub action_seq: Arc<AtomicUsize>,
    /// Set when Yemot reports an expired / unverified session, so the run can
    /// stop with `session_expired` instead of looping on failures.
    pub session_error: Arc<Mutex<Option<YemotError>>>,
    /// Files the user attached to THIS run — the only paths an upload may read.
    pub attachments: Arc<Vec<Attachment>>,
    /// `action_id` → the attachment it uploads, filled in when the action is
    /// proposed and read back by `approve_actions` at apply time.
    pub audio: Arc<Mutex<HashMap<String, Attachment>>>,
}

impl ToolCtx {
    async fn note_yemot_error(&self, e: &YemotError) {
        if matches!(e, YemotError::MfaRequired | YemotError::SessionExpired(_)) {
            let mut slot = self.session_error.lock().await;
            if slot.is_none() {
                *slot = Some(e.clone());
            }
        }
    }

    async fn next_action_id(&self) -> String {
        format!("a_{}", self.action_seq.fetch_add(1, Ordering::SeqCst) + 1)
    }
}

// ---------------------------------------------------------------------------
// Result capping
// ---------------------------------------------------------------------------

/// Cap a tool result at `MAX_RESULT_TOKENS` — the backstop when no per-tool
/// cap applies. Always cuts on a whole-char boundary.
#[allow(dead_code)] // the dispatcher goes through `cap_result_to` + `cap_for`
pub fn cap_result(text: &str) -> String {
    cap_result_to(text, MAX_RESULT_TOKENS)
}

/// Cap a tool result at `max` tokens, on a whole-char boundary and with a note
/// the model can act on.
pub fn cap_result_to(text: &str, max: usize) -> String {
    let max = max.min(MAX_RESULT_TOKENS);
    if knowledge::estimate_tokens(text) <= max {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut keep = chars.len();
    // Shrink geometrically, then walk back to a line break for readability.
    while keep > 0 {
        let candidate: String = chars[..keep].iter().collect();
        if knowledge::estimate_tokens(&candidate) <= max.saturating_sub(40) {
            let cut = candidate.rfind('\n').map(|i| i + 1).unwrap_or(candidate.len());
            let mut out: String = candidate[..cut].to_string();
            out.push_str("\n... [קוצר: התוצאה ארוכה מהמותר. צמצם את החיפוש או בקש סעיף מסוים]\n");
            return out;
        }
        keep = keep * 9 / 10;
    }
    "... [קוצר]\n".to_string()
}

// ---------------------------------------------------------------------------
// Labels
// ---------------------------------------------------------------------------

fn s(input: &Value, key: &str) -> String {
    input.get(key).and_then(|v| v.as_str()).unwrap_or("").trim().to_string()
}

/// Short Hebrew label for `agent:tool_started`.
pub fn label_for(name: &str, input: &Value) -> String {
    match name {
        "search_knowledge" => format!("חיפוש ידע: {}", s(input, "query")),
        "get_knowledge_section" => {
            let h = s(input, "heading");
            if h.is_empty() {
                format!("קריאת מסמך: {}", s(input, "file"))
            } else {
                format!("קריאת סעיף: {} › {}", s(input, "file"), h)
            }
        }
        "lookup_param" => format!("בדיקת הגדרה: {}", s(input, "key")),
        "get_extension_config" => format!("קריאת שלוחה: {}", s(input, "path")),
        "list_extensions" => format!("רשימת שלוחות: {}", s(input, "path")),
        "list_files" => format!("רשימת קבצים: {}", s(input, "path")),
        "get_text_file" => format!("קריאת קובץ: {}", s(input, "path")),
        "get_system_info" => "פרטי מערכת".to_string(),
        "set_extension_params" => format!("כתיבת הגדרות: {}", s(input, "path")),
        "upload_text_file" => format!("כתיבת קובץ: {}", s(input, "path")),
        "upload_audio_file" => format!("העלאת שמע: {}", s(input, "dest_path")),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Dispatcher
// ---------------------------------------------------------------------------

pub async fn execute_tool(
    name: &str,
    input: &Value,
    tool_use_id: &str,
    ctx: &ToolCtx,
) -> ToolOutcome {
    if DENIED_TOOLS.contains(&name) {
        return ToolOutcome::err(format!(
            "TOOL_NOT_AVAILABLE: הכלי {} אינו זמין ביישום הזה. אל תנסה שוב.",
            name
        ));
    }
    if let Some(raw) = input.get(INVALID_ARGUMENTS_KEY).and_then(|v| v.as_str()) {
        return ToolOutcome::err(format!(
            "ARGUMENTS_PARSE_ERROR: הפרמטרים שנשלחו אינם JSON תקין ({}). שלח את הקריאה שוב עם JSON תקין.",
            raw.chars().take(120).collect::<String>()
        ));
    }
    if spec_of(name).is_none() {
        return ToolOutcome::err(format!(
            "TOOL_NOT_AVAILABLE: אין כלי בשם {}. השתמש רק בכלים שהוגדרו.",
            name
        ));
    }

    let out = match name {
        "search_knowledge" => {
            let q = s(input, "query");
            if q.is_empty() {
                ToolOutcome::err("query ריק")
            } else {
                ToolOutcome::ok(knowledge::search_knowledge(&q, SearchOpts::default()))
            }
        }
        "get_knowledge_section" => {
            let file = s(input, "file");
            let heading = input.get("heading").and_then(|v| v.as_str()).map(str::trim);
            let heading = heading.filter(|h| !h.is_empty());
            match knowledge::get_knowledge_section(&file, heading, None) {
                Ok(t) => ToolOutcome::ok(t),
                Err(e) => ToolOutcome::err(e),
            }
        }
        "lookup_param" => {
            let key = s(input, "key");
            if key.is_empty() {
                ToolOutcome::err("key ריק")
            } else {
                ToolOutcome::ok(knowledge::lookup_param(&key))
            }
        }
        "get_extension_config" => read_extension(ctx, &s(input, "path")).await,
        "list_extensions" => {
            let path = s(input, "path");
            let path = if path.is_empty() { "/".to_string() } else { path };
            let depth = input.get("depth").and_then(|v| v.as_u64()).unwrap_or(1).clamp(1, 2) as u8;
            match ctx.client.list_extensions(&path, depth).await {
                Ok(nodes) => ToolOutcome::ok(yemot::render_tree(&nodes)),
                Err(e) => {
                    ctx.note_yemot_error(&e).await;
                    ToolOutcome::err(yemot::render_error(&e))
                }
            }
        }
        "list_files" => {
            let path = s(input, "path");
            let path = if path.is_empty() { "/".to_string() } else { path };
            match ctx.client.list_files(&path).await {
                Ok(files) => ToolOutcome::ok(yemot::render_files(&path, &files)),
                Err(e) => {
                    ctx.note_yemot_error(&e).await;
                    ToolOutcome::err(yemot::render_error(&e))
                }
            }
        }
        "get_text_file" => {
            let path = s(input, "path");
            match ctx.client.get_text_file(&path).await {
                Ok(f) if !f.exists => ToolOutcome::ok(format!("file={} exists=false\n", path)),
                Ok(f) => ToolOutcome::ok(format!(
                    "file={} exists=true\n{}\n",
                    path,
                    f.contents.trim_end()
                )),
                Err(e) => {
                    ctx.note_yemot_error(&e).await;
                    ToolOutcome::err(yemot::render_error(&e))
                }
            }
        }
        "get_system_info" => match ctx.client.get_system_info().await {
            Ok(i) => ToolOutcome::ok(format!(
                "system={} units={} units_expire={}\n",
                i.system,
                i.units.map(|u| u.to_string()).unwrap_or_else(|| "-".into()),
                i.units_expire.unwrap_or_else(|| "-".into())
            )),
            Err(e) => {
                ctx.note_yemot_error(&e).await;
                ToolOutcome::err(yemot::render_error(&e))
            }
        },
        "set_extension_params" => set_extension_params(ctx, input, tool_use_id).await,
        "upload_text_file" => upload_text_file(ctx, input, tool_use_id).await,
        "upload_audio_file" => upload_audio_file(ctx, input, tool_use_id).await,
        _ => ToolOutcome::err("TOOL_NOT_AVAILABLE"),
    };

    ToolOutcome {
        content: cap_result_to(&out.content, cap_for(name)),
        is_error: out.is_error,
    }
}

async fn read_extension(ctx: &ToolCtx, path: &str) -> ToolOutcome {
    let canon = match yemot::canon_ext(path) {
        Ok(c) => c,
        Err(e) => return ToolOutcome::err(yemot::render_error(&e)),
    };
    // A `Listing` snapshot came from GetIVR2Dir, not from ext.ini itself; it may
    // be partial, so it must not open the write gate — fetch the real file.
    let read = match ctx.client.get_ext_ini(&canon).await {
        Ok(r) if r.source == ExtSource::File => Ok(r),
        Ok(_) => ctx.client.get_ext_ini_fresh(&canon).await,
        Err(e) => Err(e),
    };
    match read {
        Ok(read) => {
            let rendered = yemot::render_ext_read(&canon, &read, 8_000);
            ctx.reads.lock().await.insert(canon.clone());
            ctx.ext_reads.lock().await.insert(canon, read);
            ToolOutcome::ok(rendered)
        }
        Err(e) => {
            ctx.note_yemot_error(&e).await;
            ToolOutcome::err(yemot::render_error(&e))
        }
    }
}

// ---------------------------------------------------------------------------
// Writes
// ---------------------------------------------------------------------------

/// The synthetic result of a deferred write. Must stay byte-identical for the
/// same inputs — it is part of the cached prefix of every later turn.
pub fn pending_result(display_path: &str, keys: &[String]) -> String {
    format!(
        "status=pending_approval ext={} keys={}\n{}",
        display_path,
        keys.join(","),
        PENDING_NOTE
    )
}

fn parse_params(input: &Value) -> Result<Vec<(String, String)>, String> {
    let arr = input
        .get("params")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "params חייב להיות מערך של {key,value}".to_string())?;
    if arr.is_empty() {
        return Err("params ריק — לא נשלחו הגדרות לכתיבה".to_string());
    }
    let mut out = Vec::new();
    for p in arr {
        let key = p.get("key").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        let value = p.get("value").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if key.is_empty() {
            return Err("שם הגדרה ריק ב-params".to_string());
        }
        if key.eq_ignore_ascii_case("token") || key.eq_ignore_ascii_case("path") {
            return Err(format!("שם פרמטר שמור ואינו מותר: {}", key));
        }
        if key.contains(['\r', '\n', '=']) {
            return Err(format!("שם הגדרה לא חוקי: {}", key));
        }
        if value.contains(['\r', '\n']) {
            return Err(format!("ערך רב-שורתי אינו נתמך בעדכון שלוחה: {}", key));
        }
        out.push((key, value));
    }
    Ok(out)
}

async fn set_extension_params(ctx: &ToolCtx, input: &Value, tool_use_id: &str) -> ToolOutcome {
    let raw_path = s(input, "path");
    let canon = match yemot::canon_ext(&raw_path) {
        Ok(c) => c,
        Err(e) => return ToolOutcome::err(yemot::render_error(&e)),
    };
    let display = yemot::display_path(&canon);

    let params = match parse_params(input) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::err(e),
    };

    // Read-before-write: without the current file a merge is a guess. The read
    // itself is reused — `get_extension_config` already paid for it.
    let read = match ctx.ext_reads.lock().await.get(&canon).cloned() {
        Some(r) => r,
        None => {
            return ToolOutcome::err(
                "READ_FIRST_REQUIRED: קרא קודם get_extension_config עבור נתיב זה".to_string(),
            )
        }
    };
    if !read.exists && !params.iter().any(|(k, _)| k.eq_ignore_ascii_case("type")) {
        return ToolOutcome::err(format!(
            "TYPE_REQUIRED: השלוחה {} אינה קיימת, ולכן חובה לשלוח גם type=",
            display
        ));
    }

    if ctx.auto_apply {
        return match ctx.client.update_extension(&canon, &params).await {
            Ok(outcomes) => ToolOutcome::ok(yemot::render_outcomes(&canon, &outcomes)),
            Err(e) => {
                ctx.note_yemot_error(&e).await;
                ToolOutcome::err(yemot::render_error(&e))
            }
        };
    }

    let diff = read.ini.preview_merge(&params);
    let mut warnings: Vec<String> = Vec::new();
    for (k, v) in &params {
        if !knowledge::is_known_param(k) {
            warnings.push(format!("מפתח לא מתועד: {}", k));
        }
        if k.eq_ignore_ascii_case("type") && !knowledge::is_known_type(v) {
            warnings.push(format!("סוג שלוחה לא מוכר: type={}", v));
        }
    }

    // Changing `type` re-purposes an existing extension: the keys of the old
    // type stay in the file and mostly stop meaning anything.
    let type_change = read.exists
        && params
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("type"))
            .zip(read.ini.get("type"))
            .map(|((_, new), old)| new.trim() != old.trim())
            .unwrap_or(false);
    if type_change {
        let new_type = params
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("type"))
            .map(|(_, v)| v.trim().to_string())
            .unwrap_or_default();
        warnings.push(format!(
            "שינוי סוג השלוחה מ-{} ל-{}: ההגדרות של הסוג הקודם נשארות בקובץ",
            read.ini.get("type").unwrap_or("-"),
            new_type
        ));
        // `knowledge` exposes no per-type parameter index (only the global
        // `is_known_param`), so this lists the existing keys that are not
        // documented at all — the closest signal available without one.
        let mut stale: Vec<&str> = read
            .ini
            .pairs()
            .into_iter()
            .map(|(k, _)| k)
            .filter(|k| {
                !k.eq_ignore_ascii_case("type")
                    && !k.eq_ignore_ascii_case("title")
                    && !params.iter().any(|(pk, _)| pk == k)
                    && !knowledge::is_known_param(k)
            })
            .collect();
        stale.dedup();
        if !stale.is_empty() {
            warnings.push(format!(
                "מפתחות קיימים שאינם מתועדים במאגר — בדוק אם הם רלוונטיים לסוג החדש: {}",
                stale.join(", ")
            ));
        }
    }

    let risk = if type_change {
        "destructive"
    } else if diff.iter().any(|d| d.kind == crate::yemot_ini::DiffKind::Changed) {
        "overwrite"
    } else {
        "low"
    };

    let action = ProposedAction {
        id: ctx.next_action_id().await,
        tool_use_id: tool_use_id.to_string(),
        kind: "set_extension_params".to_string(),
        path: display.clone(),
        canon_path: canon.clone(),
        params: params
            .iter()
            .map(|(k, v)| ActionParam { key: k.clone(), value: v.clone() })
            .collect(),
        contents: None,
        reason: s(input, "reason"),
        risk: risk.to_string(),
        exists: read.exists,
        diff: diff.iter().map(DiffRowDto::from).collect(),
        warnings,
        previous: None,
        snapshot_hash: Some(read.snapshot_hash()),
    };

    let keys: Vec<String> = params.iter().map(|(k, _)| k.clone()).collect();
    events::action_proposed(&ctx.app, &ctx.run_id, &action);
    ctx.proposed.lock().await.push(action);
    ToolOutcome::ok(pending_result(&display, &keys))
}

/// Is this canonical path an extension's `ext.ini`?
pub fn is_ext_ini(canon_path: &str) -> bool {
    canon_path
        .rsplit('/')
        .next()
        .map(|n| n.eq_ignore_ascii_case("ext.ini"))
        .unwrap_or(false)
}

async fn upload_text_file(ctx: &ToolCtx, input: &Value, tool_use_id: &str) -> ToolOutcome {
    let raw_path = s(input, "path");
    let canon = match yemot::canon_file(&raw_path) {
        Ok(c) => c,
        Err(e) => return ToolOutcome::err(yemot::render_error(&e)),
    };
    let contents = input.get("contents").and_then(|v| v.as_str()).unwrap_or("").to_string();

    // `UploadTextFile` replaces the whole file. On ext.ini that erases every
    // setting the tool never saw — no read gate, no diff, no per-key merge.
    if is_ext_ini(&canon) {
        return ToolOutcome::err(
            "EXT_INI_NOT_ALLOWED: אסור לכתוב ל-ext.ini עם upload_text_file — הכתיבה דורסת את כל הגדרות השלוחה. השתמש ב-set_extension_params, שממזג רק את המפתחות שאתה שולח."
                .to_string(),
        );
    }

    if ctx.auto_apply {
        return match ctx.client.upload_text_file(&canon, &contents).await {
            Ok(_) => ToolOutcome::ok(format!("upload {} ok bytes={}\n", canon, contents.len())),
            Err(e) => {
                ctx.note_yemot_error(&e).await;
                ToolOutcome::err(yemot::render_error(&e))
            }
        };
    }

    // Read the file first so the approval dialog can show a real before/after
    // and `undo_action` has something to restore. A *failed* read is not the
    // same as "no such file": folding the two together would show the user a
    // low-risk "new file" for a write that in fact destroys existing content.
    let previous = match ctx.client.get_text_file(&canon).await {
        Ok(f) if f.exists => Some(f.contents),
        Ok(_) => None,
        Err(e) => {
            ctx.note_yemot_error(&e).await;
            return ToolOutcome::err(format!(
                "READ_FAILED: לא ניתן לקרוא את {} לפני הכתיבה, ולכן אי אפשר להציע החלפה של הקובץ. {}",
                canon,
                yemot::render_error(&e)
            ));
        }
    };
    let exists = previous.is_some();
    let risk = match &previous {
        Some(p) if !p.trim().is_empty() => "destructive",
        Some(_) => "overwrite",
        None => "low",
    };

    let action = ProposedAction {
        id: ctx.next_action_id().await,
        tool_use_id: tool_use_id.to_string(),
        kind: "upload_text_file".to_string(),
        path: canon.clone(),
        canon_path: canon.clone(),
        params: Vec::new(),
        contents: Some(contents.clone()),
        reason: s(input, "reason"),
        risk: risk.to_string(),
        exists,
        diff: vec![DiffRowDto {
            key: "contents".to_string(),
            before: previous.clone(),
            after: contents,
            kind: if exists { "changed" } else { "new" }.to_string(),
        }],
        warnings: Vec::new(),
        // Same stale guard the extension writes get: the approval re-reads the
        // file and refuses if the server no longer holds what the diff showed.
        snapshot_hash: Some(yemot::content_hash(
            exists,
            previous.as_deref().unwrap_or(""),
        )),
        previous,
    };
    events::action_proposed(&ctx.app, &ctx.run_id, &action);
    ctx.proposed.lock().await.push(action);
    ToolOutcome::ok(pending_result(&canon, &["contents".to_string()]))
}

/// `ivr2:/1/000.wav` → (`ivr2:/1`, `000.wav`).
fn dir_and_name(canon_file_path: &str) -> (String, String) {
    match canon_file_path.rsplit_once('/') {
        Some(("ivr2:", name)) => ("ivr2:/".to_string(), name.to_string()),
        Some((dir, name)) => (dir.to_string(), name.to_string()),
        None => ("ivr2:/".to_string(), canon_file_path.to_string()),
    }
}

fn kb(bytes: u64) -> String {
    format!("{} KB", bytes.div_ceil(1024))
}

/// Shape one audio upload proposal. `existing` is the name the destination
/// already holds (`None` = nothing there), which is the only thing that decides
/// the risk and the "this cannot be undone" warning.
fn audio_action(
    id: &str,
    tool_use_id: &str,
    canon: &str,
    att: &Attachment,
    existing: Option<String>,
    reason: &str,
) -> ProposedAction {
    let exists = existing.is_some();
    ProposedAction {
        id: id.to_string(),
        tool_use_id: tool_use_id.to_string(),
        kind: "upload_audio_file".to_string(),
        path: canon.to_string(),
        canon_path: canon.to_string(),
        params: vec![
            ActionParam { key: "file".to_string(), value: att.name.clone() },
            ActionParam { key: "size".to_string(), value: kb(att.size) },
        ],
        contents: None,
        reason: reason.to_string(),
        risk: if exists { "overwrite" } else { "low" }.to_string(),
        exists,
        diff: vec![DiffRowDto {
            key: "file".to_string(),
            before: existing,
            after: att.name.clone(),
            kind: if exists { "changed" } else { "new" }.to_string(),
        }],
        // Audio cannot be restored: Yemot exposes no file delete and the old
        // recording is gone once the upload lands.
        warnings: if exists {
            vec!["לא ניתן לבטל העלאת שמע אוטומטית".to_string()]
        } else {
            Vec::new()
        },
        previous: None,
        snapshot_hash: None,
    }
}

/// Upload one of the user's attached audio files.
///
/// The model never names a local path: it names an id from `[קבצים מצורפים]`,
/// and the id is resolved against this run's own attachment list. An id that is
/// not on that list is an error, not a lookup elsewhere.
async fn upload_audio_file(ctx: &ToolCtx, input: &Value, tool_use_id: &str) -> ToolOutcome {
    if ctx.attachments.is_empty() {
        return ToolOutcome::err("אין קבצים מצורפים למשימה");
    }
    let id = s(input, "attachment_id");
    let Some(att) = ctx.attachments.iter().find(|a| a.id == id).cloned() else {
        let ids: Vec<&str> = ctx.attachments.iter().map(|a| a.id.as_str()).collect();
        return ToolOutcome::err(format!(
            "ATTACHMENT_NOT_FOUND: אין קובץ מצורף במזהה {}. המזהים הזמינים: {}",
            id,
            ids.join(", ")
        ));
    };

    let canon = match yemot::canon_file(&s(input, "dest_path")) {
        Ok(c) => c,
        Err(e) => return ToolOutcome::err(yemot::render_error(&e)),
    };
    let (dir, name) = dir_and_name(&canon);
    if !name.to_lowercase().ends_with(".wav") {
        return ToolOutcome::err(format!(
            "DEST_MUST_BE_WAV: נתיב היעד חייב להסתיים ב-.wav (המערכת ממירה את {} בעצמה). שלח למשל {}/000.wav",
            att.name,
            yemot::display_path(&dir)
        ));
    }

    if ctx.auto_apply {
        let bytes = match std::fs::read(&att.local_path) {
            Ok(b) => b,
            Err(_) => {
                return ToolOutcome::err(format!("לא ניתן לקרוא את הקובץ {} מהמחשב", att.name))
            }
        };
        return match ctx.client.upload_audio_file(&canon, bytes, &att.name).await {
            Ok(size) => ToolOutcome::ok(format!("upload {} ok bytes={}\n", canon, size)),
            Err(e) => {
                ctx.note_yemot_error(&e).await;
                ToolOutcome::err(yemot::render_error(&e))
            }
        };
    }

    // Does the destination already hold a file of that name? Fails closed: an
    // unknown answer would show the user a "new file" for what may be an
    // irreversible overwrite.
    let existing = match ctx.client.list_files(&dir).await {
        Ok(files) => files
            .into_iter()
            .find(|f| f.name.eq_ignore_ascii_case(&name))
            .map(|f| f.name),
        Err(e) => {
            ctx.note_yemot_error(&e).await;
            return ToolOutcome::err(format!(
                "READ_FAILED: לא ניתן לבדוק אם {} כבר קיים, ולכן אי אפשר להציע העלאה. {}",
                canon,
                yemot::render_error(&e)
            ));
        }
    };
    let action = audio_action(
        &ctx.next_action_id().await,
        tool_use_id,
        &canon,
        &att,
        existing,
        &s(input, "reason"),
    );

    // The bytes stay on disk until the approval; only the mapping is kept.
    ctx.audio.lock().await.insert(action.id.clone(), att);
    events::action_proposed(&ctx.app, &ctx.run_id, &action);
    ctx.proposed.lock().await.push(action);
    ToolOutcome::ok(pending_result(&canon, &["file".to_string()]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_schema_is_strict_compatible() {
        fn walk(v: &Value) {
            if let Some(o) = v.as_object() {
                if o.get("type").and_then(|t| t.as_str()) == Some("object") {
                    assert_eq!(
                        o.get("additionalProperties"),
                        Some(&json!(false)),
                        "object schema without additionalProperties:false"
                    );
                    let props: Vec<&String> = o
                        .get("properties")
                        .and_then(|p| p.as_object())
                        .map(|p| p.keys().collect())
                        .unwrap_or_default();
                    let required: Vec<String> = o
                        .get("required")
                        .and_then(|r| r.as_array())
                        .map(|r| r.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                        .unwrap_or_default();
                    for p in &props {
                        assert!(required.contains(p), "strict mode: {} not in required", p);
                    }
                }
                for banned in ["minimum", "maximum", "default"] {
                    assert!(o.get(banned).is_none(), "banned keyword {}", banned);
                }
                for sub in o.values() {
                    walk(sub);
                }
            }
        }
        let names: Vec<&str> = tool_specs().iter().map(|t| t.name).collect();
        assert!(names.contains(&"list_files"), "new tools must be covered here");
        assert!(names.contains(&"upload_audio_file"), "new tools must be covered here");
        for t in tool_specs() {
            walk(&t.input_schema);
        }
    }

    /// The tool list is part of the cached prefix: it must not depend on
    /// whether this particular run has attachments.
    #[test]
    fn the_audio_tool_is_always_present_and_the_list_stays_byte_stable() {
        let json = |_: ()| {
            serde_json::to_string(&super::super::providers::anthropic::tools_json(&tool_specs()))
                .unwrap()
        };
        assert_eq!(json(()), json(()));
        let spec = spec_of("upload_audio_file").expect("always exposed");
        let props = spec.input_schema["properties"].as_object().unwrap();
        let mut keys: Vec<&String> = props.keys().collect();
        keys.sort();
        assert_eq!(keys, vec!["attachment_id", "dest_path", "reason"]);
    }

    #[test]
    fn tts_files_are_explained_in_the_text_upload_description() {
        let d = spec_of("upload_text_file").unwrap().description;
        assert!(d.contains(".tts"), "the tts sentence is missing");
        assert!(d.contains("text to speech"));
    }

    fn attachment() -> Attachment {
        Attachment {
            id: "f1".into(),
            name: "ברכה.mp3".into(),
            local_path: "C:/tmp/ברכה.mp3".into(),
            size: 2_048,
            mime: "audio/mpeg".into(),
        }
    }

    #[test]
    fn audio_action_is_low_risk_on_a_free_slot_and_overwrite_on_a_taken_one() {
        let att = attachment();
        let fresh = audio_action("a_1", "toolu_1", "ivr2:/1/000.wav", &att, None, "ברכה");
        assert_eq!(fresh.kind, "upload_audio_file");
        assert_eq!(fresh.risk, "low");
        assert!(!fresh.exists);
        assert!(fresh.warnings.is_empty());
        assert!(fresh.contents.is_none());
        assert!(fresh.previous.is_none());
        assert!(fresh.snapshot_hash.is_none());
        assert_eq!(fresh.diff[0].kind, "new");
        assert_eq!(fresh.diff[0].before, None);
        assert_eq!(fresh.diff[0].after, "ברכה.mp3");
        assert_eq!(fresh.params[0].value, "ברכה.mp3");
        assert_eq!(fresh.params[1].value, "2 KB");

        let over = audio_action(
            "a_2",
            "toolu_2",
            "ivr2:/1/000.wav",
            &att,
            Some("000.wav".to_string()),
            "החלפה",
        );
        assert_eq!(over.risk, "overwrite");
        assert!(over.exists);
        assert_eq!(over.diff[0].kind, "changed");
        assert_eq!(over.diff[0].before.as_deref(), Some("000.wav"));
        assert_eq!(over.warnings, vec!["לא ניתן לבטל העלאת שמע אוטומטית"]);
    }

    #[test]
    fn upload_destination_splits_into_folder_and_name() {
        assert_eq!(
            dir_and_name("ivr2:/1/2/000.wav"),
            ("ivr2:/1/2".to_string(), "000.wav".to_string())
        );
        assert_eq!(
            dir_and_name("ivr2:/000.wav"),
            ("ivr2:/".to_string(), "000.wav".to_string())
        );
        assert_eq!(kb(1), "1 KB");
        assert_eq!(kb(1024), "1 KB");
        assert_eq!(kb(1025), "2 KB");
    }

    #[test]
    fn tool_inventory_and_kinds() {
        let names: Vec<&str> = tool_specs().iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            vec![
                "search_knowledge",
                "get_knowledge_section",
                "lookup_param",
                "get_extension_config",
                "list_extensions",
                "list_files",
                "get_text_file",
                "get_system_info",
                "set_extension_params",
                "upload_text_file",
                "upload_audio_file",
            ]
        );
        assert!(is_mutating("set_extension_params"));
        assert!(is_mutating("upload_text_file"));
        assert!(is_mutating("upload_audio_file"));
        assert!(!is_mutating("search_knowledge"));
        assert!(!is_mutating("nope"));
    }

    #[test]
    fn descriptions_stay_short() {
        for t in tool_specs() {
            let words = t.description.split_whitespace().count();
            assert!(words <= 70, "{} description is {} words", t.name, words);
            assert!(words >= 15, "{} description is too thin", t.name);
        }
    }

    #[test]
    fn specs_are_byte_stable() {
        let a = serde_json::to_string(&super::super::providers::anthropic::tools_json(&tool_specs())).unwrap();
        let b = serde_json::to_string(&super::super::providers::anthropic::tools_json(&tool_specs())).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn pending_result_is_byte_identical() {
        let keys = vec!["type".to_string(), "title".to_string()];
        let a = pending_result("/3", &keys);
        let b = pending_result("/3", &keys);
        assert_eq!(a, b);
        assert_eq!(
            a,
            "status=pending_approval ext=/3 keys=type,title\nנרשמה לאישור, טרם בוצעה. המשך לשלוחה הבאה."
        );
    }

    #[test]
    fn results_are_capped_with_a_note() {
        let short = "ext=/3 exists=true\ntype=menu\n";
        assert_eq!(cap_result(short), short);

        let long = "type=menu title=בדיקה ארוכה מאוד\n".repeat(4000);
        let capped = cap_result(&long);
        assert!(capped.len() < long.len());
        assert!(knowledge::estimate_tokens(&capped) <= MAX_RESULT_TOKENS);
        assert!(capped.contains("[קוצר"));
    }

    #[test]
    fn every_tool_has_its_own_cap_and_none_exceeds_the_backstop() {
        for t in tool_specs() {
            let cap = cap_for(t.name);
            assert!(cap <= MAX_RESULT_TOKENS, "{} cap above the backstop", t.name);
            if t.kind == ToolKind::ReadOnly {
                assert!(cap < MAX_RESULT_TOKENS, "{} still uses the backstop", t.name);
            }
        }
        assert_eq!(cap_for("get_system_info"), 300);
        assert_eq!(cap_for("search_knowledge"), 1_200);
        assert_eq!(cap_for("nope"), MAX_RESULT_TOKENS);

        let long = "type=menu title=בדיקה ארוכה מאוד\n".repeat(4000);
        let tight = cap_result_to(&long, cap_for("get_extension_config"));
        assert!(knowledge::estimate_tokens(&tight) <= 800);
        assert!(tight.len() < cap_result(&long).len());
    }

    #[test]
    fn ext_ini_is_recognised_case_insensitively() {
        assert!(is_ext_ini("ivr2:/1/ext.ini"));
        assert!(is_ext_ini("ivr2:/EXT.INI"));
        assert!(!is_ext_ini("ivr2:/1/list.ini"));
        assert!(!is_ext_ini("ivr2:/1/000.wav"));
    }

    #[test]
    fn params_validation() {
        assert!(parse_params(&json!({"params": []})).is_err());
        assert!(parse_params(&json!({"params": [{"key":"","value":"x"}]})).is_err());
        assert!(parse_params(&json!({"params": [{"key":"token","value":"x"}]})).is_err());
        assert!(parse_params(&json!({"params": [{"key":"a=b","value":"x"}]})).is_err());
        assert!(parse_params(&json!({"params": [{"key":"title","value":"a\nb"}]})).is_err());
        let ok = parse_params(&json!({"params": [{"key":"type","value":"menu"}]})).unwrap();
        assert_eq!(ok, vec![("type".to_string(), "menu".to_string())]);
    }

    #[test]
    fn denied_tools_cover_the_dangerous_surface() {
        for n in [
            "file_action", "delete_extension", "run_tzintuk", "run_campaign",
            "schedule_campaign", "send_sms", "send_fax", "transfer_units",
            "set_password", "set_customer_details", "kill_session", "call_action",
        ] {
            assert!(DENIED_TOOLS.contains(&n));
            assert!(spec_of(n).is_none(), "{} must not be an exposed spec", n);
        }
    }

    #[test]
    fn labels_are_hebrew_and_short() {
        assert_eq!(
            label_for("search_knowledge", &json!({"query": "תפריט"})),
            "חיפוש ידע: תפריט"
        );
        assert_eq!(
            label_for("get_extension_config", &json!({"path": "/3"})),
            "קריאת שלוחה: /3"
        );
        assert_eq!(label_for("get_system_info", &json!({})), "פרטי מערכת");
    }

    #[test]
    fn summary_is_first_line_and_bounded() {
        let o = ToolOutcome::ok("ext=/3 exists=true\ntype=menu\ntitle=x\n");
        assert_eq!(o.summary(), "ext=/3 exists=true");
        let long = ToolOutcome::ok("א".repeat(400));
        assert!(long.summary().chars().count() <= 120);
    }
}
