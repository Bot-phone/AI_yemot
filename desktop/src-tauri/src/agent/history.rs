//! Local task history ("היסטוריית משימות").
//!
//! A **task** is one chain: a fresh run plus every refinement of it. The
//! `task_id` is the `run_id` of the first run in the chain, and every finished
//! run of the chain rewrites the same record — the newest transcript wins, the
//! list of instructions grows.
//!
//! One JSON file per task at `<app_data_dir>/tasks/<task_id>.json`, written
//! atomically (`<file>.tmp` + rename). Retention is applied on every save:
//! at most `MAX_TASKS` records and `MAX_BYTES` in total, oldest by
//! `updated_at_ms` first. A corrupt or unreadable file is skipped and logged,
//! never fatal — history is a convenience, not a dependency of the run.
//!
//! Privacy: the record holds the transcript, the proposals and the attachment
//! list. It never holds the Yemot token, a provider API key or the system
//! prompt. It stays on this machine.

use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::{IgnoredAny, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use tauri::{AppHandle, Manager};

use super::events::ProposedAction;
use super::runner::Attachment;
use super::types::{Message, Usage};

// ---------------------------------------------------------------------------
// Retention
// ---------------------------------------------------------------------------

/// At most this many task files are kept.
pub const MAX_TASKS: usize = 200;
/// …and at most this many bytes in total.
pub const MAX_BYTES: u64 = 100 * 1024 * 1024;
/// `TaskSummary::title` never exceeds this many *characters* (not bytes).
pub const TITLE_MAX_CHARS: usize = 80;

/// The one error the user ever sees from this module.
pub const NOT_FOUND_MSG: &str = "המשימה לא נמצאה בהיסטוריה";

// ---------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------

/// Token / turn / cost totals across the whole chain, not just the last run.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub turns: u32,
    pub tool_calls: u32,
    pub cost_usd: Option<f64>,
}

impl TaskUsage {
    /// Fold one finished run into the chain totals. A run whose model is not in
    /// the price table contributes no cost, and a chain with no priced run at
    /// all keeps `cost_usd: None` rather than reporting a misleading `$0`.
    pub fn add_run(&mut self, usage: &Usage, turns: u32, tool_calls: u32, cost: Option<f64>) {
        self.input_tokens += usage.input;
        self.output_tokens += usage.output;
        self.cache_read_tokens += usage.cache_read;
        self.cache_write_tokens += usage.cache_write;
        self.turns += turns;
        self.tool_calls += tool_calls;
        if let Some(c) = cost {
            self.cost_usd = Some(self.cost_usd.unwrap_or(0.0) + c);
        }
    }
}

/// One saved task. Written whole on every save; read back by
/// `continue_agent_run` to resume a task the app has since forgotten.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRecord {
    pub task_id: String,
    pub last_run_id: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub provider: String,
    /// The resolved model id (never `regular` / `pro`).
    pub model: String,
    /// The chain of instructions, oldest first.
    pub instructions: Vec<String>,
    /// The last run's final answer.
    pub final_text: String,
    pub ok: bool,
    /// The last run's stop reason.
    pub stop: String,
    /// The last run's full transcript — what a continuation replays.
    pub transcript: Vec<Message>,
    pub attachments: Vec<Attachment>,
    /// The last run's proposals.
    pub actions: Vec<ProposedAction>,
    /// Ids of those proposals that were applied and not undone.
    pub applied_action_ids: Vec<String>,
    pub usage: TaskUsage,
}

/// One row of "היסטוריית משימות". Display only — never carries a transcript.
#[derive(Debug, Clone, Serialize)]
pub struct TaskSummary {
    pub task_id: String,
    pub last_run_id: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub provider: String,
    pub model: String,
    /// The first instruction of the chain, at most `TITLE_MAX_CHARS` chars.
    pub title: String,
    /// How many instructions the chain holds ("N שלבים").
    pub steps: usize,
    pub ok: bool,
    pub stop: String,
    pub cost_usd: Option<f64>,
    /// A task can only be continued while it still has a transcript to replay.
    pub resumable: bool,
}

/// What the live chain carries between the runs of one task. Held by both the
/// `RunHandle` (a continuation of a *live* parent reads it) and the run
/// context (the save point writes it).
#[derive(Debug, Clone, Default)]
pub struct ChainState {
    pub task_id: String,
    pub created_at_ms: u64,
    pub instructions: Vec<String>,
    pub usage: TaskUsage,
}

// ---------------------------------------------------------------------------
// Header — the cheap read used by listing and pruning
// ---------------------------------------------------------------------------

/// Length of a JSON array, counted without materializing it: listing 200 tasks
/// must not allocate 200 transcripts to learn whether they are empty.
#[derive(Debug, Clone, Copy, Default)]
struct SeqLen(usize);

impl<'de> Deserialize<'de> for SeqLen {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = SeqLen;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an array")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<SeqLen, A::Error> {
                let mut n = 0usize;
                while seq.next_element::<IgnoredAny>()?.is_some() {
                    n += 1;
                }
                Ok(SeqLen(n))
            }
        }
        d.deserialize_seq(V)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct TaskHeader {
    task_id: String,
    #[serde(default)]
    last_run_id: String,
    #[serde(default)]
    created_at_ms: u64,
    #[serde(default)]
    updated_at_ms: u64,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    instructions: Vec<String>,
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    stop: String,
    #[serde(default)]
    transcript: SeqLen,
    #[serde(default)]
    usage: TaskUsage,
}

impl From<TaskHeader> for TaskSummary {
    fn from(h: TaskHeader) -> Self {
        TaskSummary {
            title: title_of(&h.instructions),
            steps: h.instructions.len(),
            resumable: h.transcript.0 > 0,
            task_id: h.task_id,
            last_run_id: h.last_run_id,
            created_at_ms: h.created_at_ms,
            updated_at_ms: h.updated_at_ms,
            provider: h.provider,
            model: h.model,
            ok: h.ok,
            stop: h.stop,
            cost_usd: h.usage.cost_usd,
        }
    }
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// A task id is a run id (`r_<hex>`), and it is used as a file name — so it is
/// checked, not trusted. Anything else cannot escape the tasks directory.
pub fn valid_task_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// `<app_data_dir>/tasks`, created if missing.
pub fn tasks_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("לא נמצאה תיקיית הנתונים של היישום: {}", e))?;
    let dir = base.join("tasks");
    fs::create_dir_all(&dir).map_err(|e| format!("לא ניתן ליצור את תיקיית ההיסטוריה: {}", e))?;
    Ok(dir)
}

fn file_of(dir: &Path, task_id: &str) -> Result<PathBuf, String> {
    if !valid_task_id(task_id) {
        return Err(NOT_FOUND_MSG.to_string());
    }
    Ok(dir.join(format!("{}.json", task_id)))
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The first instruction, trimmed to one line and at most `TITLE_MAX_CHARS`
/// characters — cut on a character boundary, so Hebrew never breaks mid-byte.
pub fn title_of(instructions: &[String]) -> String {
    let first = instructions.first().map(|s| s.as_str()).unwrap_or("");
    let line: String = first
        .split(['\n', '\r'])
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string();
    if line.chars().count() <= TITLE_MAX_CHARS {
        return line;
    }
    let mut out: String = line.chars().take(TITLE_MAX_CHARS - 1).collect();
    out.push('…');
    out
}

// ---------------------------------------------------------------------------
// Read
// ---------------------------------------------------------------------------

/// Every readable header in the directory, with the file it came from and its
/// size on disk. Unreadable / corrupt files are logged and skipped.
fn headers(dir: &Path) -> Vec<(PathBuf, u64, TaskHeader)> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        match fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|s| {
            serde_json::from_str::<TaskHeader>(&s).map_err(|e| e.to_string())
        }) {
            Ok(h) => out.push((path, size, h)),
            Err(e) => eprintln!("task history: מדלג על {} — {}", path.display(), e),
        }
    }
    out
}

/// Every saved task, newest first by `updated_at_ms`.
pub fn list(dir: &Path) -> Vec<TaskSummary> {
    let mut rows: Vec<TaskSummary> = headers(dir).into_iter().map(|(_, _, h)| h.into()).collect();
    rows.sort_by(|a, b| {
        b.updated_at_ms
            .cmp(&a.updated_at_ms)
            .then_with(|| b.task_id.cmp(&a.task_id))
    });
    rows
}

pub fn load(dir: &Path, task_id: &str) -> Result<TaskRecord, String> {
    let path = file_of(dir, task_id)?;
    let raw = fs::read_to_string(&path).map_err(|_| NOT_FOUND_MSG.to_string())?;
    serde_json::from_str(&raw).map_err(|e| {
        eprintln!("task history: {} פגום — {}", path.display(), e);
        NOT_FOUND_MSG.to_string()
    })
}

/// Resolve the parent of a continuation: `id` is either a `task_id` (the file
/// name) or the `run_id` of the chain's newest run.
pub fn load_for_parent(dir: &Path, id: &str) -> Option<TaskRecord> {
    if let Ok(rec) = load(dir, id) {
        return Some(rec);
    }
    let task_id = headers(dir)
        .into_iter()
        .find(|(_, _, h)| h.last_run_id == id)
        .map(|(_, _, h)| h.task_id)?;
    load(dir, &task_id).ok()
}

// ---------------------------------------------------------------------------
// Write
// ---------------------------------------------------------------------------

/// The one gate on writing a record: nothing is stored when the user turned
/// history off, and a run with no transcript has nothing to resume from.
pub fn should_save(save_history: bool, transcript: &[Message]) -> bool {
    save_history && !transcript.is_empty()
}

/// Write the record (atomically) and apply retention. A no-op when the gate
/// says not to save — an existing record for that chain is then left as is.
pub fn save_if(dir: &Path, save_history: bool, rec: &TaskRecord) -> Result<(), String> {
    if !should_save(save_history, &rec.transcript) {
        return Ok(());
    }
    save(dir, rec)
}

pub fn save(dir: &Path, rec: &TaskRecord) -> Result<(), String> {
    let path = file_of(dir, &rec.task_id)?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string(rec).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json.as_bytes()).map_err(|e| e.to_string())?;
    // Atomic on both platforms we ship: `rename` replaces an existing file.
    if let Err(e) = fs::rename(&tmp, &path) {
        let _ = fs::remove_file(&tmp);
        return Err(e.to_string());
    }
    prune(dir);
    Ok(())
}

/// Trim the directory to `MAX_TASKS` files and `MAX_BYTES`, oldest
/// `updated_at_ms` first. Leftover `.tmp` files from a crashed write go too.
pub fn prune(dir: &Path) {
    prune_with(dir, MAX_TASKS, MAX_BYTES);
}

/// `prune` with explicit budgets, so the retention rule can be tested without
/// writing 100 MiB to disk.
fn prune_with(dir: &Path, max_tasks: usize, max_bytes: u64) {
    let mut rows = headers(dir);
    // newest first — everything past the budget is dropped
    rows.sort_by(|a, b| {
        b.2.updated_at_ms
            .cmp(&a.2.updated_at_ms)
            .then_with(|| b.2.task_id.cmp(&a.2.task_id))
    });
    let mut bytes: u64 = 0;
    for (i, (path, size, _)) in rows.iter().enumerate() {
        bytes = bytes.saturating_add(*size);
        if i >= max_tasks || bytes > max_bytes {
            if let Err(e) = fs::remove_file(path) {
                eprintln!("task history: לא ניתן למחוק {} — {}", path.display(), e);
            }
        }
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("tmp") {
                let _ = fs::remove_file(&path);
            }
        }
    }
}

/// Deleting an unknown task is a no-op, not an error.
pub fn delete(dir: &Path, task_id: &str) -> Result<(), String> {
    let path = file_of(dir, task_id)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("לא ניתן למחוק את המשימה: {}", e)),
    }
}

/// Remove every saved task; returns how many files were removed.
pub fn clear(dir: &Path) -> usize {
    let mut removed = 0usize;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str());
            if ext != Some("json") && ext != Some("tmp") {
                continue;
            }
            match fs::remove_file(&path) {
                Ok(()) if ext == Some("json") => removed += 1,
                Ok(()) => {}
                Err(e) => eprintln!("task history: לא ניתן למחוק {} — {}", path.display(), e),
            }
        }
    }
    removed
}

/// Rewrite only the applied-ids of the chain whose newest run is `run_id`.
/// Called after an approval or an undo, so a resumed task reports
/// בוצע / לא אושר correctly instead of the empty set the run ended with.
pub fn set_applied(dir: &Path, run_id: &str, applied: Vec<String>) {
    let Some(mut rec) = load_for_parent(dir, run_id) else {
        return; // no record (history off, or pruned) — nothing to update
    };
    if rec.last_run_id != run_id || rec.applied_action_ids == applied {
        return;
    }
    rec.applied_action_ids = applied;
    rec.updated_at_ms = now_ms();
    if let Err(e) = save(dir, &rec) {
        eprintln!("task history: עדכון סטטוס הביצוע נכשל — {}", e);
    }
}

// ---------------------------------------------------------------------------
// Async wrappers — a history write must never fail or delay a run
// ---------------------------------------------------------------------------

/// Save off the async runtime. Every failure is logged and swallowed: the run
/// has already produced its result, and losing the record must not change it.
pub async fn save_task(app: &AppHandle, save_history: bool, rec: TaskRecord) {
    let dir = match tasks_dir(app) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("task history: {}", e);
            return;
        }
    };
    let _ = tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = save_if(&dir, save_history, &rec) {
            eprintln!("task history: שמירת המשימה נכשלה — {}", e);
        }
    })
    .await;
}

/// Same, for the applied-ids refresh after an approval / undo.
pub async fn sync_applied(app: &AppHandle, run_id: &str, applied: Vec<String>) {
    let dir = match tasks_dir(app) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("task history: {}", e);
            return;
        }
    };
    let run_id = run_id.to_string();
    let _ = tauri::async_runtime::spawn_blocking(move || set_applied(&dir, &run_id, applied)).await;
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_task_history(app: AppHandle) -> Result<Vec<TaskSummary>, String> {
    let dir = tasks_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || list(&dir))
        .await
        .map_err(|e| format!("קריאת ההיסטוריה נכשלה: {}", e))
}

/// One task for display. The transcript is deliberately **not** returned: it is
/// large, the UI never renders it, and `continue_agent_run` reads it in Rust.
#[tauri::command]
pub async fn get_task_history(app: AppHandle, task_id: String) -> Result<TaskRecord, String> {
    let dir = tasks_dir(&app)?;
    let mut rec = tauri::async_runtime::spawn_blocking(move || load(&dir, &task_id))
        .await
        .map_err(|e| format!("קריאת המשימה נכשלה: {}", e))??;
    rec.transcript = Vec::new();
    Ok(rec)
}

#[tauri::command]
pub async fn delete_task_history(app: AppHandle, task_id: String) -> Result<(), String> {
    let dir = tasks_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || delete(&dir, &task_id))
        .await
        .map_err(|e| format!("מחיקת המשימה נכשלה: {}", e))?
}

#[tauri::command]
pub async fn clear_task_history(app: AppHandle) -> Result<usize, String> {
    let dir = tasks_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || clear(&dir))
        .await
        .map_err(|e| format!("ניקוי ההיסטוריה נכשל: {}", e))
}

/// The applied set of a run, as the status block and the record report it:
/// claimed ids minus the ones an undo reversed.
pub fn applied_ids(applied: &HashSet<String>, undone: &HashSet<String>) -> Vec<String> {
    let mut ids: Vec<String> = applied.difference(undone).cloned().collect();
    ids.sort();
    ids
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::events::{ActionParam, DiffRowDto};
    use crate::agent::types::{ContentBlock, Role};

    /// A throwaway directory under the OS temp dir; removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> TempDir {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let dir = std::env::temp_dir().join(format!("ai_yemot_hist_{}_{:x}", tag, nanos));
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn action(id: &str) -> ProposedAction {
        ProposedAction {
            id: id.to_string(),
            tool_use_id: format!("toolu_{}", id),
            kind: "set_extension_params".into(),
            path: "/3".into(),
            canon_path: "ivr2:/3".into(),
            params: vec![ActionParam { key: "type".into(), value: "menu".into() }],
            contents: None,
            reason: "הגדרת שלוחה 3 כתפריט".into(),
            risk: "low".into(),
            exists: true,
            diff: vec![DiffRowDto {
                key: "type".into(),
                before: Some("playfile".into()),
                after: "menu".into(),
                kind: "changed".into(),
            }],
            warnings: vec![],
            previous: None,
            snapshot_hash: Some("true:1234".into()),
        }
    }

    fn record(task_id: &str, updated: u64) -> TaskRecord {
        TaskRecord {
            task_id: task_id.to_string(),
            last_run_id: format!("{}_last", task_id),
            created_at_ms: 1,
            updated_at_ms: updated,
            provider: "claude".into(),
            model: "claude-sonnet-4-5".into(),
            instructions: vec!["הפוך את שלוחה 3 לתפריט".into()],
            final_text: "בוצע".into(),
            ok: true,
            stop: "end_turn".into(),
            transcript: vec![
                Message::user_text("הפוך את שלוחה 3 לתפריט"),
                Message {
                    role: Role::Assistant,
                    content: vec![
                        ContentBlock::Text("קורא את השלוחה".into()),
                        ContentBlock::ToolUse {
                            id: "toolu_1".into(),
                            name: "get_extension_config".into(),
                            input: serde_json::json!({ "path": "/3" }),
                        },
                    ],
                },
                Message {
                    role: Role::User,
                    content: vec![ContentBlock::ToolResult {
                        tool_use_id: "toolu_1".into(),
                        name: "get_extension_config".into(),
                        content: "type=playfile".into(),
                        is_error: false,
                    }],
                },
            ],
            attachments: vec![Attachment {
                id: "f1".into(),
                name: "ברכה.mp3".into(),
                local_path: "C:/tmp/ברכה.mp3".into(),
                size: 20480,
                mime: "audio/mpeg".into(),
            }],
            actions: vec![action("a_1"), action("a_2")],
            applied_action_ids: vec!["a_1".into()],
            usage: TaskUsage {
                input_tokens: 10,
                output_tokens: 20,
                cache_read_tokens: 30,
                cache_write_tokens: 40,
                turns: 2,
                tool_calls: 3,
                cost_usd: Some(0.25),
            },
        }
    }

    #[test]
    fn a_record_round_trips_through_disk() {
        let dir = TempDir::new("round");
        let rec = record("r_1", 100);
        save(dir.path(), &rec).unwrap();
        let back = load(dir.path(), "r_1").unwrap();

        assert_eq!(back.task_id, "r_1");
        assert_eq!(back.instructions, rec.instructions);
        assert_eq!(back.transcript.len(), 3);
        assert_eq!(back.actions.len(), 2);
        assert_eq!(back.applied_action_ids, vec!["a_1".to_string()]);
        assert_eq!(back.usage, rec.usage);
        // the content blocks survive their tagged wire form
        match &back.transcript[1].content[1] {
            ContentBlock::ToolUse { name, input, .. } => {
                assert_eq!(name, "get_extension_config");
                assert_eq!(input["path"], serde_json::json!("/3"));
            }
            other => panic!("wrong block: {:?}", other),
        }
        match &back.transcript[2].content[0] {
            ContentBlock::ToolResult { content, is_error, .. } => {
                assert_eq!(content, "type=playfile");
                assert!(!is_error);
            }
            other => panic!("wrong block: {:?}", other),
        }
        // the tagged shape itself is the contract the frontend may read
        let v = serde_json::to_value(&rec.transcript[1]).unwrap();
        assert_eq!(v["role"], serde_json::json!("assistant"));
        assert_eq!(v["content"][0]["type"], serde_json::json!("text"));
        assert_eq!(v["content"][1]["type"], serde_json::json!("tool_use"));
    }

    #[test]
    fn the_record_holds_no_secret_and_one_local_path() {
        let dir = TempDir::new("privacy");
        let rec = record("r_1", 100);
        save(dir.path(), &rec).unwrap();
        let raw = fs::read_to_string(dir.path().join("r_1.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();

        // no secret material anywhere in the file
        for forbidden in ["yemot_token", "api_key", "base_url", "system"] {
            assert!(
                !raw.contains(forbidden),
                "the record must not carry {}",
                forbidden
            );
        }
        // and exactly one local path: the attachment's
        assert_eq!(raw.matches("local_path").count(), 1);
        assert_eq!(
            v["attachments"][0]["local_path"],
            serde_json::json!("C:/tmp/ברכה.mp3")
        );
    }

    #[test]
    fn the_list_is_newest_first_and_carries_no_transcript() {
        let dir = TempDir::new("list");
        save(dir.path(), &record("r_old", 100)).unwrap();
        save(dir.path(), &record("r_new", 300)).unwrap();
        save(dir.path(), &record("r_mid", 200)).unwrap();

        let rows = list(dir.path());
        let ids: Vec<&str> = rows.iter().map(|r| r.task_id.as_str()).collect();
        assert_eq!(ids, vec!["r_new", "r_mid", "r_old"]);

        let first = &rows[0];
        assert_eq!(first.title, "הפוך את שלוחה 3 לתפריט");
        assert_eq!(first.steps, 1);
        assert!(first.resumable);
        assert_eq!(first.cost_usd, Some(0.25));
        assert_eq!(first.last_run_id, "r_new_last");
        // a summary is display-only: no transcript field at all
        let v = serde_json::to_value(first).unwrap();
        assert!(v.get("transcript").is_none());
        for f in [
            "task_id", "last_run_id", "created_at_ms", "updated_at_ms", "provider", "model",
            "title", "steps", "ok", "stop", "cost_usd", "resumable",
        ] {
            assert!(v.get(f).is_some(), "missing summary field {}", f);
        }
    }

    #[test]
    fn a_task_with_no_transcript_is_not_resumable() {
        let dir = TempDir::new("resumable");
        let mut rec = record("r_1", 100);
        rec.transcript.clear();
        // written directly: `save_if` would refuse an empty transcript
        save(dir.path(), &rec).unwrap();
        assert!(!list(dir.path())[0].resumable);
    }

    #[test]
    fn delete_and_clear_remove_records() {
        let dir = TempDir::new("delete");
        save(dir.path(), &record("r_1", 100)).unwrap();
        save(dir.path(), &record("r_2", 200)).unwrap();

        delete(dir.path(), "r_1").unwrap();
        assert_eq!(list(dir.path()).len(), 1);
        // deleting what is not there is what the user meant anyway
        delete(dir.path(), "r_1").unwrap();
        // …and a traversal attempt is simply "not found", never a write
        assert!(delete(dir.path(), "../../evil").is_err());

        assert_eq!(clear(dir.path()), 1);
        assert!(list(dir.path()).is_empty());
        assert_eq!(clear(dir.path()), 0);
    }

    #[test]
    fn a_corrupt_file_is_skipped_not_fatal() {
        let dir = TempDir::new("corrupt");
        save(dir.path(), &record("r_1", 100)).unwrap();
        fs::write(dir.path().join("r_bad.json"), "{ not json").unwrap();
        let rows = list(dir.path());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].task_id, "r_1");
        assert!(load(dir.path(), "r_bad").is_err());
    }

    #[test]
    fn pruning_keeps_the_newest_by_count() {
        let dir = TempDir::new("count");
        for i in 0..(MAX_TASKS + 5) {
            save(dir.path(), &record(&format!("r_{:04}", i), i as u64)).unwrap();
        }
        let rows = list(dir.path());
        assert_eq!(rows.len(), MAX_TASKS);
        assert_eq!(rows[0].task_id, format!("r_{:04}", MAX_TASKS + 4));
        assert!(load(dir.path(), "r_0000").is_err(), "the oldest is gone");
    }

    #[test]
    fn pruning_keeps_the_newest_by_bytes() {
        let dir = TempDir::new("bytes");
        // three fat records, then a budget that only two of them fit into
        let big = "x".repeat(64 * 1024);
        for i in 0..3u64 {
            let mut rec = record(&format!("r_{}", i), i);
            rec.final_text = big.clone();
            save(dir.path(), &rec).unwrap();
        }
        let one = headers(dir.path())[0].1;
        assert!(one > 64 * 1024, "the fat record really is fat");

        prune_with(dir.path(), MAX_TASKS, one * 2 + 1);
        let ids: Vec<String> = list(dir.path()).into_iter().map(|r| r.task_id).collect();
        assert_eq!(ids, vec!["r_2".to_string(), "r_1".to_string()]);
    }

    #[test]
    fn pruning_sweeps_a_leftover_tmp_file() {
        let dir = TempDir::new("tmp");
        save(dir.path(), &record("r_1", 100)).unwrap();
        fs::write(dir.path().join("r_9.json.tmp"), "half written").unwrap();
        prune(dir.path());
        assert!(!dir.path().join("r_9.json.tmp").exists());
        assert_eq!(list(dir.path()).len(), 1);
    }

    #[test]
    fn a_title_is_cut_on_a_hebrew_character_boundary() {
        // 100 two-byte Hebrew characters: a byte-wise cut at 80 would split one
        let long: String = "ש".repeat(100);
        let title = title_of(&[long]);
        assert_eq!(title.chars().count(), TITLE_MAX_CHARS);
        assert!(title.ends_with('…'));
        assert!(title.is_char_boundary(title.len()));
        // short instructions are kept verbatim, first non-empty line only
        assert_eq!(title_of(&["\n  שלוחה 3  \nעוד".into()]), "שלוחה 3");
        assert_eq!(title_of(&[]), "");
    }

    #[test]
    fn history_off_writes_nothing_and_leaves_an_existing_record() {
        let dir = TempDir::new("off");
        let rec = record("r_1", 100);
        save_if(dir.path(), true, &rec).unwrap();
        assert_eq!(list(dir.path()).len(), 1);

        let mut later = record("r_1", 999);
        later.final_text = "לא אמור להישמר".into();
        save_if(dir.path(), false, &later).unwrap();
        assert_eq!(load(dir.path(), "r_1").unwrap().updated_at_ms, 100);

        // …and an empty transcript is never saved either
        let mut empty = record("r_2", 100);
        empty.transcript.clear();
        save_if(dir.path(), true, &empty).unwrap();
        assert!(load(dir.path(), "r_2").is_err());
        assert!(!should_save(true, &[]));
        assert!(!should_save(false, &rec.transcript));
        assert!(should_save(true, &rec.transcript));
    }

    #[test]
    fn a_parent_resolves_by_task_id_or_by_last_run_id() {
        let dir = TempDir::new("parent");
        save(dir.path(), &record("r_1", 100)).unwrap();
        assert_eq!(load_for_parent(dir.path(), "r_1").unwrap().task_id, "r_1");
        assert_eq!(
            load_for_parent(dir.path(), "r_1_last").unwrap().task_id,
            "r_1"
        );
        assert!(load_for_parent(dir.path(), "r_nope").is_none());
    }

    #[test]
    fn applied_ids_are_refreshed_after_an_approval() {
        let dir = TempDir::new("applied");
        let mut rec = record("r_1", 100);
        rec.applied_action_ids.clear();
        save(dir.path(), &rec).unwrap();

        // by the chain's newest run id, which is what approve_actions holds
        set_applied(dir.path(), "r_1_last", vec!["a_1".into(), "a_2".into()]);
        assert_eq!(
            load(dir.path(), "r_1").unwrap().applied_action_ids,
            vec!["a_1".to_string(), "a_2".to_string()]
        );
        // an undo removes the id again
        set_applied(dir.path(), "r_1_last", vec!["a_2".into()]);
        assert_eq!(
            load(dir.path(), "r_1").unwrap().applied_action_ids,
            vec!["a_2".to_string()]
        );
        // an unknown run touches nothing
        set_applied(dir.path(), "r_nope", vec!["a_9".into()]);
        assert_eq!(
            load(dir.path(), "r_1").unwrap().applied_action_ids,
            vec!["a_2".to_string()]
        );
    }

    #[test]
    fn chain_usage_sums_runs_and_keeps_an_unpriced_chain_unpriced() {
        let mut u = TaskUsage::default();
        u.add_run(
            &Usage { input: 1, output: 2, cache_read: 3, cache_write: 4 },
            2,
            1,
            None,
        );
        assert_eq!(u.cost_usd, None);
        u.add_run(
            &Usage { input: 10, output: 20, cache_read: 30, cache_write: 40 },
            3,
            2,
            Some(0.5),
        );
        assert_eq!(u.input_tokens, 11);
        assert_eq!(u.turns, 5);
        assert_eq!(u.tool_calls, 3);
        assert_eq!(u.cost_usd, Some(0.5));
    }

    #[test]
    fn a_task_id_must_be_a_file_name() {
        assert!(valid_task_id("r_18f3a2b"));
        assert!(!valid_task_id(""));
        assert!(!valid_task_id("../evil"));
        assert!(!valid_task_id("r_1/../../x"));
        assert!(!valid_task_id("r_1.json"));
        assert!(!valid_task_id(&"r".repeat(65)));
    }

    #[test]
    fn applied_ids_drop_the_undone_ones() {
        let applied = HashSet::from(["a_1".to_string(), "a_2".to_string()]);
        let undone = HashSet::from(["a_1".to_string()]);
        assert_eq!(applied_ids(&applied, &undone), vec!["a_2".to_string()]);
    }
}
