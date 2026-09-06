//! The agent loop and its three Tauri commands.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, State};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::yemot::{self, ParamOutcome, YemotClient};

use super::events::{
    self, ActionApplyResult, ActionParam, AgentRunStarted, ProposedAction, RunUsage,
};
use super::history::{self, ChainState, TaskRecord};
use super::prompt;
use super::providers::{self, Provider};
use super::tools::{self, ToolCtx, ToolOutcome};
use super::types::{ContentBlock, Message, ProviderError, ProviderRequest, Role, StopReason, Usage};
use super::{pricing, types::ProviderResponse};

// ---------------------------------------------------------------------------
// Budgets
// ---------------------------------------------------------------------------

const MAX_TURNS: u32 = 12;
const MAX_TOOL_CALLS_PER_TURN: usize = 8;
const RUN_BUDGET: Duration = Duration::from_secs(600);
const MAX_ATTEMPTS: u32 = 4;
const TREE_LINES: usize = 20;

/// Above this the run stops rather than paying for a prompt that no longer
/// fits its purpose. History is never rewritten: collapsing old tool results
/// invalidates the cached prefix, and re-reading the whole prompt uncached
/// costs more than the tokens it saves.
pub const CONTEXT_HARD_LIMIT: usize = 150_000;

/// The same call, a third time, is a loop — not a retry.
pub const LOOP_LIMIT: u32 = 2;
/// After this many blocked repeats the run is not going to recover.
pub const MAX_LOOP_BLOCKS: u32 = 5;
pub const LOOP_MSG: &str =
    "LOOP_DETECTED: קריאה זהה כבר בוצעה פעמיים. השתמש בתוצאה הקודמת או שנה גישה.";

// ---------------------------------------------------------------------------
// Loop detection
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct LoopGuard {
    seen: HashMap<String, u32>,
    blocked: u32,
}

impl LoopGuard {
    /// `name + input`, with any `path` canonicalized so `/3`, `ivr2:/3` and
    /// `/3/` count as the same call. `serde_json::Map` is a BTreeMap, so the
    /// serialization is key-sorted and stable.
    pub fn signature(name: &str, input: &Value) -> String {
        let mut v = input.clone();
        if let Some(obj) = v.as_object_mut() {
            if let Some(p) = obj.get("path").and_then(|p| p.as_str()) {
                let canon = yemot::canon_ext(p)
                    .or_else(|_| yemot::canon_file(p))
                    .unwrap_or_else(|_| p.trim().to_string());
                obj.insert("path".to_string(), Value::String(canon));
            }
        }
        format!("{}#{}", name, v)
    }

    /// `true` when this exact call has already run `LOOP_LIMIT` times.
    pub fn check(&mut self, name: &str, input: &Value) -> bool {
        let sig = LoopGuard::signature(name, input);
        let count = self.seen.entry(sig).or_insert(0);
        *count += 1;
        if *count > LOOP_LIMIT {
            self.blocked += 1;
            true
        } else {
            false
        }
    }

    pub fn blocked(&self) -> u32 {
        self.blocked
    }

    pub fn exhausted(&self) -> bool {
        self.blocked >= MAX_LOOP_BLOCKS
    }
}

// ---------------------------------------------------------------------------
// Context size
// ---------------------------------------------------------------------------

/// Rough prompt size in tokens: system blocks + every message block.
pub fn estimate_context(system: &[String], messages: &[Message]) -> usize {
    let mut total: usize = system.iter().map(|s| crate::knowledge::estimate_tokens(s)).sum();
    for m in messages {
        for b in &m.content {
            total += match b {
                ContentBlock::Text(t) => crate::knowledge::estimate_tokens(t),
                ContentBlock::ToolResult { content, .. } => {
                    crate::knowledge::estimate_tokens(content)
                }
                ContentBlock::ToolUse { input, .. } => {
                    crate::knowledge::estimate_tokens(&input.to_string())
                }
                ContentBlock::Thinking(raw) => crate::knowledge::estimate_tokens(&raw.to_string()),
            };
        }
    }
    total
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct RunHandle {
    pub cancel: CancellationToken,
    pub proposed: Arc<Mutex<Vec<ProposedAction>>>,
    pub client: Arc<YemotClient>,
    /// Set when `run_loop` returns. The handle deliberately stays in the
    /// registry afterwards: approval happens *after* `agent:finished`, and it
    /// needs both the proposed actions and the authenticated Yemot client.
    pub finished: Arc<AtomicBool>,
    /// The messages the loop ended with — the starting point of a continuation.
    /// It lives here and nowhere else: the registry evicts a finished handle as
    /// soon as the next run claims a slot, so this cannot grow.
    pub transcript: Arc<Mutex<Vec<Message>>>,
    /// Files the *user* attached to this run. An `upload_audio_file` action
    /// resolves its attachment id against this list only — never against
    /// anything the model wrote.
    pub audio: Arc<Mutex<HashMap<String, Attachment>>>,
    /// The run's attachment list, exactly as the model was shown it. A
    /// continuation replays this run's transcript — which still advertises
    /// these ids — so it must inherit the list or those ids stop resolving.
    pub attachments: Arc<Vec<Attachment>>,
    /// The task this run belongs to: its id, its instruction chain and the
    /// usage totals of every run in it. A continuation of a *live* parent
    /// takes the chain from here; a continuation of a forgotten one reads the
    /// same fields back from the saved record.
    pub chain: Arc<Mutex<ChainState>>,
}

impl RunHandle {
    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::SeqCst)
    }
}

#[derive(Default)]
pub struct AgentRegistry {
    pub runs: Mutex<HashMap<String, RunHandle>>,
}

impl AgentRegistry {
    /// Register a new run. Rejected only while an *unfinished* run exists;
    /// handles of finished runs are evicted here, which is what bounds the map.
    pub async fn claim(&self, run_id: &str, handle: RunHandle) -> Result<(), String> {
        let mut runs = self.runs.lock().await;
        if runs.values().any(|h| !h.is_finished()) {
            return Err("ריצה אחרת פעילה כרגע. בטל אותה לפני התחלת ריצה חדשה.".to_string());
        }
        runs.clear(); // whatever is left has finished — its actions are stale now
        runs.insert(run_id.to_string(), handle);
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentRunPayload {
    pub provider: String,
    pub model: String,
    pub prompt: String,
    pub api_key: String,
    #[serde(default)]
    pub base_url: String,
    /// Held privately in Rust; never serialized outward.
    pub yemot_token: String,
    #[serde(default)]
    pub auto_apply: bool,
    #[serde(default)]
    pub include_tree: bool,
    /// Audio files the user picked for this task. Absent = none.
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    /// Keep this task in the local history ("היסטוריית משימות"). Default on;
    /// when off nothing is written and an existing record for the chain is
    /// left exactly as it was.
    #[serde(default = "default_true")]
    pub save_history: bool,
}

fn default_true() -> bool {
    true
}

/// One file the user attached to a task. `local_path` stays in Rust: it is the
/// only source an `upload_audio_file` may read from, and it is never sent to the
/// model (which sees the id, name, size and mime).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub local_path: String,
    pub size: u64,
    pub mime: String,
}

/// Audio formats Yemot's `UploadFile` converts (`convertAudio=1`).
pub const AUDIO_EXTS: &[&str] = &["wav", "mp3", "m4a", "ogg", "wma", "aac"];

fn ext_of(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, e)| e.trim().to_lowercase())
        .unwrap_or_default()
}

/// KB, rounded up, so a 300-byte file is not reported as `0 KB`.
fn size_kb(bytes: u64) -> u64 {
    bytes.div_ceil(1024)
}

fn too_big_msg(name: &str) -> String {
    format!(
        "הקובץ {} גדול מ-{} MB",
        name,
        yemot::MAX_UPLOAD_BYTES / (1024 * 1024)
    )
}

/// Validate the user's attachments at run start, and **write the real size on
/// disk back into each one** — the size the frontend declares is advisory (it
/// is `0` when the file was picked through the native dialog), and it is what
/// the model, the proposal params and the change log all quote.
///
/// Fails closed: an attachment that cannot be checked (missing, not a regular
/// file) is an error, not a warning — the run would otherwise propose an upload
/// it can never apply.
pub fn validate_attachments(list: &mut [Attachment]) -> Result<(), String> {
    let mut seen: HashSet<String> = HashSet::new();
    for a in list.iter_mut() {
        if a.id.trim().is_empty() || !seen.insert(a.id.clone()) {
            return Err(format!("מזהה קובץ מצורף כפול או ריק: {}", a.id));
        }
        // Both names are checked: the display name is what the user sees, the
        // local path is what actually gets read and uploaded, and a run must
        // not be able to upload a `.exe` that merely *claims* to be a `.mp3`.
        for candidate in [&a.name, &a.local_path] {
            let ext = ext_of(candidate);
            if !AUDIO_EXTS.contains(&ext.as_str()) {
                return Err(format!(
                    "הקובץ {} אינו קובץ שמע נתמך. סוגים נתמכים: {}",
                    a.name,
                    AUDIO_EXTS.join(", ")
                ));
            }
        }
        let mime = a.mime.trim().to_lowercase();
        if !mime.is_empty() && !mime.starts_with("audio/") {
            return Err(format!("סוג הקובץ {} אינו שמע: {}", a.name, a.mime));
        }
        match std::fs::metadata(&a.local_path) {
            Ok(m) if !m.is_file() => {
                return Err(format!("הנתיב של {} אינו קובץ", a.name));
            }
            Ok(m) if m.len() > yemot::MAX_UPLOAD_BYTES => return Err(too_big_msg(&a.name)),
            // The disk is the authority on size; the declared value is dropped.
            Ok(m) => a.size = m.len(),
            Err(_) => return Err(format!("הקובץ {} לא נמצא במחשב", a.name)),
        }
    }
    Ok(())
}

/// Re-check one attachment at *apply* time. The approval dialog may sit open
/// for minutes, so the file that `validate_attachments` measured can have been
/// replaced by a bigger one — or by a directory — before the bytes are read.
fn recheck_attachment(att: &Attachment) -> Result<(), String> {
    match std::fs::metadata(&att.local_path) {
        Ok(m) if !m.is_file() => Err(format!("הנתיב של {} אינו קובץ", att.name)),
        Ok(m) if m.len() > yemot::MAX_UPLOAD_BYTES => Err(too_big_msg(&att.name)),
        Ok(_) => Ok(()),
        Err(_) => Err(format!("לא ניתן לקרוא את הקובץ {} מהמחשב", att.name)),
    }
}

/// `[קבצים מצורפים]` — the only place the model learns which ids exist.
/// Empty list = no block at all, so a run without attachments is byte-identical
/// to one from before this feature.
pub fn attachments_block(list: &[Attachment]) -> Option<String> {
    if list.is_empty() {
        return None;
    }
    let mut out = String::from("[קבצים מצורפים]\n");
    for a in list {
        out.push_str(&format!(
            "- {}: {} ({} KB, {})\n",
            a.id,
            a.name,
            size_kb(a.size),
            a.mime
        ));
    }
    Some(out)
}

fn new_run_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("r_{:x}", nanos)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn start_agent_run(
    app: AppHandle,
    registry: State<'_, AgentRegistry>,
    mut payload: AgentRunPayload,
) -> Result<AgentRunStarted, String> {
    if payload.prompt.trim().is_empty() {
        return Err("לא הוזנה בקשה".to_string());
    }
    if payload.yemot_token.trim().is_empty() {
        return Err("נדרשת התחברות למערכת ימות המשיח".to_string());
    }
    validate_attachments(&mut payload.attachments)?;
    let provider = providers::build(
        &payload.provider,
        &payload.model,
        &payload.api_key,
        &payload.base_url,
    )?;
    let model = providers::resolve_model(&payload.provider, &payload.model);

    let client = Arc::new(YemotClient::new(payload.yemot_token.clone()));
    // A fresh task: `new_run` stamps the chain's `task_id` with this run's id.
    let chain = ChainState {
        task_id: String::new(),
        created_at_ms: history::now_ms(),
        instructions: vec![payload.prompt.clone()],
        usage: Default::default(),
    };
    let (ctx, finished) = new_run(&app, &payload, model, client.clone(), registry, chain).await?;
    let run_id = ctx.run_id.clone();
    let task_id = ctx.task_id.clone();

    let prompt = payload.prompt.clone();
    let include_tree = payload.include_tree;
    let attachments = payload.attachments.clone();
    tauri::async_runtime::spawn(async move {
        // The handle is NOT removed: approval arrives after `agent:finished`
        // and still needs it. Marking it finished frees the slot; the next run
        // evicts it. The flag is set from `Drop`, so a panic (or a dropped
        // task) inside `run_loop` cannot leave the slot occupied forever.
        let _done = FinishGuard(finished);
        let initial = first_messages(&client, &prompt, include_tree, &attachments).await;
        run_loop(ctx, provider, initial).await;
    });

    Ok(AgentRunStarted { run_id, task_id })
}

/// Continue a finished task: same conversation, one more instruction.
///
/// The system blocks are regenerated (they are built from consts, so they are
/// byte-identical and the provider's cached prefix still hits) and the parent's
/// whole transcript is replayed, so nothing is re-read and nothing is re-priced
/// at full rate. `include_tree` is ignored — the tree is already in there.
#[tauri::command]
pub async fn continue_agent_run(
    app: AppHandle,
    registry: State<'_, AgentRegistry>,
    mut payload: AgentRunPayload,
    parent_run_id: String,
) -> Result<AgentRunStarted, String> {
    if payload.prompt.trim().is_empty() {
        return Err("לא הוזנה בקשה".to_string());
    }
    if payload.yemot_token.trim().is_empty() {
        return Err("נדרשת התחברות למערכת ימות המשיח".to_string());
    }
    validate_attachments(&mut payload.attachments)?;

    // Everything the parent leaves behind is read here, before the claim below
    // evicts its handle. A live parent is the fast path; once its handle is
    // gone (a later run claimed the slot, or the app was restarted) the same
    // material comes back from the saved task record.
    let live = {
        let runs = registry.runs.lock().await;
        match runs.get(&parent_run_id) {
            Some(p) if !p.is_finished() => {
                return Err("המשימה הקודמת עדיין רצה".to_string())
            }
            Some(p) => Some((
                p.transcript.lock().await.clone(),
                p.proposed.lock().await.clone(),
                p.attachments.as_ref().clone(),
                p.chain.lock().await.clone(),
            )),
            None => None,
        }
    };
    let (past, parent_actions, parent_attachments, mut chain, applied, undone) = match live {
        Some((transcript, actions, attachments, chain)) => {
            let (applied, undone) = {
                let map = write_state().lock().await;
                match map.get(&parent_run_id) {
                    Some(s) => (s.applied.clone(), s.undone.clone()),
                    None => (HashSet::new(), HashSet::new()),
                }
            };
            (transcript, actions, attachments, chain, applied, undone)
        }
        None => {
            let rec = history::tasks_dir(&app)
                .ok()
                .and_then(|dir| history::load_for_parent(&dir, &parent_run_id))
                .ok_or_else(|| "המשימה הקודמת אינה זמינה עוד — התחל משימה חדשה".to_string())?;
            // The record knows what was applied. An undone action is not in
            // that set, so a resumed task reports it as "לא אושר" rather
            // than "בוטל" — the write itself is already reversed either way.
            let applied: HashSet<String> = rec.applied_action_ids.iter().cloned().collect();
            let chain = ChainState {
                task_id: rec.task_id,
                created_at_ms: rec.created_at_ms,
                instructions: rec.instructions,
                usage: rec.usage,
            };
            (
                rec.transcript,
                rec.actions,
                rec.attachments,
                chain,
                applied,
                HashSet::new(),
            )
        }
    };
    if past.is_empty() {
        return Err("אין תמלול למשימה הקודמת — התחל משימה חדשה".to_string());
    }

    let system = prompt::system_blocks(&payload.provider);
    if estimate_context(&system, &past) > CONTEXT_HARD_LIMIT {
        return Err("המשימה ארוכה מדי להמשך, התחל משימה חדשה".to_string());
    }

    let provider = providers::build(
        &payload.provider,
        &payload.model,
        &payload.api_key,
        &payload.base_url,
    )?;
    let model = providers::resolve_model(&payload.provider, &payload.model);

    let status = status_block(&parent_actions, &applied, &undone);

    // The replayed transcript still advertises the parent's attachment ids, so
    // the continuation inherits its list; without this an `upload_audio_file`
    // on an id the model already knows would fail with ATTACHMENT_NOT_FOUND.
    let carried = carry_attachments(&parent_attachments, &payload.attachments);
    payload.attachments = carried.list;

    // The chain continues under the parent's task id, one instruction longer.
    chain.instructions.push(payload.prompt.clone());

    let client = Arc::new(YemotClient::new(payload.yemot_token.clone()));
    let (ctx, finished) = new_run(&app, &payload, model, client, registry, chain).await?;
    let run_id = ctx.run_id.clone();
    let task_id = ctx.task_id.clone();

    let mut initial = past;
    initial.push(Message::user_text(continuation_text(
        &status,
        &payload.prompt,
        // Only the *new* files are re-listed: the parent's block is already in
        // the replayed transcript, and repeating it would invite a duplicate.
        &carried.fresh,
        &carried.missing,
    )));

    tauri::async_runtime::spawn(async move {
        let _done = FinishGuard(finished);
        run_loop(ctx, provider, initial).await;
    });

    Ok(AgentRunStarted { run_id, task_id })
}

/// Register a run and build its context. Shared by a fresh run and a
/// continuation so the two cannot drift apart.
async fn new_run(
    app: &AppHandle,
    payload: &AgentRunPayload,
    model: String,
    client: Arc<YemotClient>,
    registry: State<'_, AgentRegistry>,
    mut chain: ChainState,
) -> Result<(RunContext, Arc<AtomicBool>), String> {
    let run_id = new_run_id();
    // The task id is the run id of the chain's *first* run.
    if chain.task_id.is_empty() {
        chain.task_id = run_id.clone();
    }
    if chain.created_at_ms == 0 {
        chain.created_at_ms = history::now_ms();
    }
    let task_id = chain.task_id.clone();
    let chain = Arc::new(Mutex::new(chain));
    let cancel = CancellationToken::new();
    let proposed = Arc::new(Mutex::new(Vec::new()));
    let transcript = Arc::new(Mutex::new(Vec::new()));
    let audio = Arc::new(Mutex::new(HashMap::new()));
    let finished = Arc::new(AtomicBool::new(false));
    let attachments = Arc::new(payload.attachments.clone());
    registry
        .claim(
            &run_id,
            RunHandle {
                cancel: cancel.clone(),
                proposed: proposed.clone(),
                client: client.clone(),
                finished: finished.clone(),
                transcript: transcript.clone(),
                audio: audio.clone(),
                attachments: attachments.clone(),
                chain: chain.clone(),
            },
        )
        .await?;

    Ok((
        RunContext {
            app: app.clone(),
            run_id,
            task_id,
            provider_name: payload.provider.clone(),
            model,
            auto_apply: payload.auto_apply,
            client,
            proposed,
            cancel,
            transcript,
            audio,
            attachments,
            chain,
            save_history: payload.save_history,
        },
        finished,
    ))
}

/// The first user message of a fresh run: the task, the `[מצב נוכחי]` tree and
/// the `[קבצים מצורפים]` list, in that order.
async fn first_messages(
    client: &YemotClient,
    prompt: &str,
    include_tree: bool,
    attachments: &[Attachment],
) -> Vec<Message> {
    let mut first = prompt.to_string();
    if include_tree {
        if let Some(block) = root_tree_block(client).await {
            first.push_str("\n\n");
            first.push_str(&block);
        }
    }
    if let Some(block) = attachments_block(attachments) {
        first.push_str("\n\n");
        first.push_str(&block);
    }
    vec![Message::user_text(first)]
}

/// What a continuation inherits from its parent's attachment list.
struct CarriedAttachments {
    /// The list the continuation runs with: the parent's still-usable files
    /// first (their ids are already in the replayed transcript), then the new
    /// ones. On an id collision the new payload wins — the user just picked
    /// that file, and the id is what the model will name.
    list: Vec<Attachment>,
    /// The `fresh` entries only, in payload order — what the continuation's
    /// `[קבצים מצורפים]` block advertises.
    fresh: Vec<Attachment>,
    /// `(id, name)` of parent files that are gone from disk. The model is told,
    /// so it stops offering to upload them.
    missing: Vec<(String, String)>,
}

/// Merge the parent run's attachments into the continuation's own.
///
/// A parent file is re-validated (which also refreshes its size): one that no
/// longer passes is dropped from the list and reported as missing instead of
/// staying in it as an upload that can never be applied.
fn carry_attachments(parent: &[Attachment], fresh: &[Attachment]) -> CarriedAttachments {
    let replaced: HashSet<&str> = fresh.iter().map(|a| a.id.as_str()).collect();
    let mut list = Vec::new();
    let mut missing = Vec::new();
    for a in parent {
        if replaced.contains(a.id.as_str()) {
            continue;
        }
        let mut one = a.clone();
        match validate_attachments(std::slice::from_mut(&mut one)) {
            Ok(()) => list.push(one),
            Err(_) => missing.push((a.id.clone(), a.name.clone())),
        }
    }
    list.extend(fresh.iter().cloned());
    CarriedAttachments { list, fresh: fresh.to_vec(), missing }
}

/// `[קבצים מצורפים שאינם זמינים עוד]` — parent files the continuation cannot
/// upload any more. Empty list = no block, so the common case is unchanged.
fn missing_attachments_block(missing: &[(String, String)]) -> Option<String> {
    if missing.is_empty() {
        return None;
    }
    let mut out = String::from("[קבצים מצורפים שאינם זמינים עוד]\n");
    for (id, name) in missing {
        out.push_str(&format!("- {}: {}\n", id, name));
    }
    Some(out)
}

/// The one user message a continuation adds: what happened to the previous
/// proposals, the new instruction, (when the user attached more files) the
/// attachment list this run may upload from, and which inherited files are
/// gone.
fn continuation_text(
    status: &str,
    prompt: &str,
    attachments: &[Attachment],
    missing: &[(String, String)],
) -> String {
    let mut out = String::from(status);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
    out.push_str(prompt.trim());
    if let Some(block) = attachments_block(attachments) {
        out.push_str("\n\n");
        out.push_str(&block);
    }
    if let Some(block) = missing_attachments_block(missing) {
        out.push_str("\n\n");
        out.push_str(&block);
    }
    out
}

/// `[מצב ההצעות הקודמות]` — one line per action of the parent run.
///
/// The truth is `WRITE_STATE`, not the proposal list: an id is `בוצע` while it
/// is claimed, `בוטל` once `undo_action` reversed it (which also releases the
/// claim), and `לא אושר` when the user never approved it.
pub fn status_block(
    actions: &[ProposedAction],
    applied: &HashSet<String>,
    undone: &HashSet<String>,
) -> String {
    let mut out = String::from("[מצב ההצעות הקודמות]\n");
    if actions.is_empty() {
        out.push_str("לא הוצעו שינויים במשימה הקודמת.\n");
        return out;
    }
    for a in actions {
        let state = if undone.contains(&a.id) {
            "בוטל"
        } else if applied.contains(&a.id) {
            "בוצע"
        } else {
            "לא אושר"
        };
        out.push_str(&format!("- {} {}: {}\n", a.id, a.path, state));
    }
    out
}

/// Marks a run finished when the loop's task ends — including when it panics or
/// the task is dropped mid-await. Without it one panic would reject every later
/// run with "ריצה אחרת פעילה כרגע" until the app restarts.
struct FinishGuard(Arc<AtomicBool>);

impl Drop for FinishGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tauri::command]
pub async fn cancel_agent_run(
    registry: State<'_, AgentRegistry>,
    run_id: String,
) -> Result<(), String> {
    let runs = registry.runs.lock().await;
    match runs.get(&run_id) {
        // Cancelling a run that already ended is what the user meant anyway.
        Some(h) if h.is_finished() => Ok(()),
        Some(h) => {
            h.cancel.cancel();
            // Free the slot immediately: a run wedged inside a call that does
            // not observe the token must never block the next one. The handle
            // itself stays, so its proposed actions remain approvable.
            h.finished.store(true, Ordering::SeqCst);
            Ok(())
        }
        None => Err("הריצה כבר הסתיימה".to_string()),
    }
}

// --- write safety: applied-once, stale-state, undo ------------------------
//
// The per-run write state lives here rather than on `RunHandle` on purpose: an
// undo must still work after the loop finished and the registry dropped the
// run, and this keeps `start_agent_run` untouched.

/// Everything needed to reverse one applied action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoRecord {
    pub action_id: String,
    /// `set_extension_params` | `upload_text_file`
    pub kind: String,
    pub path: String,
    #[serde(skip)]
    pub canon_path: String,
    /// Value each key had before the write; `None` = the key did not exist.
    pub params: Vec<(String, Option<String>)>,
    /// Previous file contents (`upload_text_file`); `None` = no file existed.
    pub contents: Option<String>,
    /// Fingerprint of the server state right *after* the apply. An undo that
    /// finds something else would overwrite whatever changed since.
    /// `None` = it could not be computed, so the undo goes ahead unchecked.
    #[serde(skip)]
    pub post_hash: Option<String>,
}

/// Refusal messages, shared by the apply and undo paths.
const STALE_MSG: &str = "הקובץ השתנה בשרת מאז ההצעה, הרץ שוב";
const UNVERIFIABLE_MSG: &str = "לא ניתן לאמת את מצב הקובץ בשרת, נסה שוב";
const UNDO_STALE_MSG: &str = "הקובץ השתנה מאז הביצוע, לא ניתן לבטל אוטומטית";
/// The destination was free when the upload was proposed and is not any more.
const AUDIO_APPEARED_MSG: &str = "הקובץ נוצר בשרת מאז ההצעה, הרץ שוב";
const CANCELLED_AUDIO_MSG: &str = "הריצה בוטלה, הרץ שוב לפני העלאת שמע";
/// The upload was proposed for an extension that still does not exist.
const AUDIO_FOLDER_MISSING_MSG: &str = "השלוחה עדיין לא קיימת — אשר קודם את יצירת השלוחה ואז את ההעלאה";

/// Split off the `upload_audio_file` actions of a cancelled run.
///
/// A cancelled run stays approvable on purpose — the user may still want the
/// `ext.ini` edits it already proposed, and every one of those can be undone.
/// An audio upload cannot: Yemot exposes no file delete, so pushing bytes on
/// behalf of a run the user stopped is not reversible. Refuse those, keep the
/// rest.
fn split_cancelled_audio(
    cancelled: bool,
    actions: Vec<ProposedAction>,
) -> (Vec<ProposedAction>, Vec<ProposedAction>) {
    if !cancelled {
        return (actions, Vec::new());
    }
    actions.into_iter().partition(|a| a.kind != "upload_audio_file")
}

/// Did the upload destination gain a file since the proposal said it was free?
/// Only asked when the proposal claimed the slot was empty: an overwrite the
/// user already saw and approved stays approved.
fn audio_slot_taken(proposed_exists: bool, present: &[String], name: &str) -> bool {
    !proposed_exists && present.iter().any(|n| n.eq_ignore_ascii_case(name))
}

struct RunWriteState {
    applied: HashSet<String>,
    undo: HashMap<String, UndoRecord>,
    /// Ids `undo_action` reversed. Kept separately from `applied` (an undo
    /// releases the claim so the action can be approved again) — it is what
    /// tells "בוטל" apart from "לא אושר" in the change log and the status block.
    undone: HashSet<String>,
    /// The change log of this run, oldest first.
    log: Vec<AppliedRecord>,
    client: Arc<YemotClient>,
    at: Instant,
}

/// One row of the change log ("יומן שינויים"), kept next to the undo records so
/// the log survives a page reload and is pruned by exactly the same rules.
#[derive(Debug, Clone)]
struct AppliedRecord {
    action_id: String,
    kind: String,
    path: String,
    canon_path: String,
    params: Vec<ActionParam>,
    applied_at_ms: u64,
    undo_available: bool,
}

/// The contract shape of one applied change.
#[derive(Debug, Clone, Serialize)]
pub struct AppliedChange {
    pub run_id: String,
    pub action_id: String,
    pub kind: String,
    pub path: String,
    pub display: String,
    pub applied_at_ms: u64,
    pub params: Vec<ActionParam>,
    pub undo_available: bool,
    pub undone: bool,
    pub label: String,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `("ivr2:/1/000.wav")` → `("000.wav", "/1")`.
fn file_and_parent(canon_path: &str) -> (String, String) {
    match canon_path.rsplit_once('/') {
        Some((dir, name)) => {
            let dir = dir.strip_suffix("ivr2:").map(|_| "ivr2:/").unwrap_or(dir);
            (name.to_string(), yemot::display_path(dir))
        }
        None => (canon_path.to_string(), "/".to_string()),
    }
}

/// Short Hebrew label for one applied change, e.g. `עדכון 3 הגדרות ב-/1/2`.
pub fn change_label(kind: &str, path: &str, canon_path: &str, params: usize) -> String {
    match kind {
        "upload_audio_file" => {
            let (name, parent) = file_and_parent(canon_path);
            format!("העלאת קובץ {} ל-{}", name, parent)
        }
        "upload_text_file" => {
            let (name, parent) = file_and_parent(canon_path);
            format!("כתיבת קובץ {} ל-{}", name, parent)
        }
        _ => match params {
            1 => format!("עדכון הגדרה אחת ב-{}", path),
            n => format!("עדכון {} הגדרות ב-{}", n, path),
        },
    }
}

/// `a_10` → `10`. Every row applied in one batch shares `applied_at_ms`, so the
/// tie-break decides their order — and a lexical one puts `a_10` before `a_2`.
fn action_seq(action_id: &str) -> u64 {
    action_id
        .rsplit_once('_')
        .and_then(|(_, n)| n.parse().ok())
        .unwrap_or(0)
}

static WRITE_STATE: std::sync::OnceLock<Mutex<HashMap<String, RunWriteState>>> =
    std::sync::OnceLock::new();

/// How long a finished run keeps its undo records.
const UNDO_RETENTION: Duration = Duration::from_secs(60 * 60);
/// Hard cap on retained runs. Each entry holds a live `YemotClient` (and so a
/// token), so age alone is not enough of a bound.
const MAX_WRITE_STATES: usize = 50;

fn write_state() -> &'static Mutex<HashMap<String, RunWriteState>> {
    WRITE_STATE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Drop every retained run's write state (undo records *and* the Yemot clients
/// they hold). Called on logout so no live token outlives the session.
pub async fn clear_write_state() {
    write_state().lock().await.clear();
}

/// Age out old entries, then trim the oldest until at most
/// `MAX_WRITE_STATES` remain.
fn prune(map: &mut HashMap<String, RunWriteState>) {
    map.retain(|_, s| s.at.elapsed() < UNDO_RETENTION);
    while map.len() > MAX_WRITE_STATES {
        let oldest = map
            .iter()
            .max_by_key(|(_, s)| s.at.elapsed())
            .map(|(k, _)| k.clone());
        match oldest {
            Some(k) => {
                map.remove(&k);
            }
            None => break,
        }
    }
}

/// Ids of `wanted` that were already applied in this run, and the ids that are
/// still free to write (which are marked applied on the spot, so two clicks of
/// "בצע" cannot both get through).
async fn claim_ids(run_id: &str, client: &Arc<YemotClient>, wanted: &[String]) -> HashSet<String> {
    let mut map = write_state().lock().await;
    prune(&mut map);
    let state = map.entry(run_id.to_string()).or_insert_with(|| RunWriteState {
        applied: HashSet::new(),
        undo: HashMap::new(),
        undone: HashSet::new(),
        log: Vec::new(),
        client: client.clone(),
        at: Instant::now(),
    });
    state.at = Instant::now();
    let mut already = HashSet::new();
    for id in wanted {
        if !state.applied.insert(id.clone()) {
            already.insert(id.clone());
        }
    }
    // The freshly refreshed run is the newest, so a second prune can only
    // evict older runs and keeps the map at MAX_WRITE_STATES after insertion.
    prune(&mut map);
    already
}

async fn record_undo(run_id: &str, records: Vec<UndoRecord>) {
    let mut map = write_state().lock().await;
    if let Some(state) = map.get_mut(run_id) {
        for r in records {
            state.undo.insert(r.action_id.clone(), r);
        }
    }
}

/// Add rows to the run's change log. Re-applying an id (after an undo) replaces
/// its row rather than adding a second one.
async fn record_applied(run_id: &str, records: Vec<AppliedRecord>) {
    let mut map = write_state().lock().await;
    if let Some(state) = map.get_mut(run_id) {
        for r in records {
            state.undone.remove(&r.action_id);
            state.log.retain(|old| old.action_id != r.action_id);
            state.log.push(r);
        }
    }
}

/// The change log across every retained run, newest first. This is what the UI
/// reads to rebuild "יומן שינויים" after a reload, so it must not depend on the
/// run handle still being in the registry.
#[tauri::command]
pub async fn list_applied_changes() -> Result<Vec<AppliedChange>, String> {
    let mut map = write_state().lock().await;
    prune(&mut map);
    let mut out: Vec<AppliedChange> = Vec::new();
    for (run_id, state) in map.iter() {
        for r in &state.log {
            let undone = state.undone.contains(&r.action_id);
            out.push(AppliedChange {
                run_id: run_id.clone(),
                action_id: r.action_id.clone(),
                kind: r.kind.clone(),
                path: r.path.clone(),
                display: yemot::display_path(&r.canon_path),
                applied_at_ms: r.applied_at_ms,
                params: r.params.clone(),
                undo_available: r.undo_available && !undone,
                undone,
                label: change_label(&r.kind, &r.path, &r.canon_path, r.params.len()),
            });
        }
    }
    out.sort_by(|a, b| {
        b.applied_at_ms
            .cmp(&a.applied_at_ms)
            .then_with(|| action_seq(&b.action_id).cmp(&action_seq(&a.action_id)))
            .then_with(|| b.action_id.cmp(&a.action_id))
    });
    Ok(out)
}

/// Release an id so it can be applied again (after a refusal, or an undo).
async fn release_ids(run_id: &str, ids: &[String]) {
    let mut map = write_state().lock().await;
    if let Some(state) = map.get_mut(run_id) {
        for id in ids {
            state.applied.remove(id);
        }
    }
}

#[tauri::command]
pub async fn approve_actions(
    app: AppHandle,
    registry: State<'_, AgentRegistry>,
    run_id: String,
    action_ids: Vec<String>,
) -> Result<Vec<ActionApplyResult>, String> {
    let (client, actions, audio, cancelled) = {
        let runs = registry.runs.lock().await;
        let handle = runs
            .get(&run_id)
            .ok_or_else(|| "הריצה כבר אינה זמינה — הרץ שוב".to_string())?;
        let all = handle.proposed.lock().await.clone();
        let wanted: HashSet<&String> = action_ids.iter().collect();
        let picked: Vec<ProposedAction> =
            all.into_iter().filter(|a| wanted.contains(&a.id)).collect();
        // The attachment behind each audio action, as the tool resolved it from
        // the run's own list. Nothing the model wrote reaches the filesystem.
        let audio = handle.audio.lock().await.clone();
        (
            handle.client.clone(),
            picked,
            audio,
            handle.cancel.is_cancelled(),
        )
    };
    if actions.is_empty() {
        return Err("לא נבחרו פעולות לביצוע".to_string());
    }

    // A cancelled run stays approvable — except for audio, the one kind with
    // no undo at all. See `split_cancelled_audio`.
    let (actions, blocked) = split_cancelled_audio(cancelled, actions);
    let blocked_results: Vec<ActionApplyResult> =
        blocked.iter().map(|a| refusal(a, CANCELLED_AUDIO_MSG)).collect();
    if actions.is_empty() {
        for r in &blocked_results {
            events::action_applied(&app, &run_id, r);
        }
        return Ok(blocked_results);
    }

    // Approving the same action twice would apply the write twice — and the
    // second time against a file the first one already changed.
    let ids: Vec<String> = actions.iter().map(|a| a.id.clone()).collect();
    let already = claim_ids(&run_id, &client, &ids).await;
    let fresh: Vec<ProposedAction> = actions
        .iter()
        .filter(|a| !already.contains(&a.id))
        .cloned()
        .collect();

    let outcome = apply_actions(&client, &fresh, &audio).await;
    let ApplyOutcome {
        mut results,
        undo,
        refused,
        rehashed,
    } = outcome;
    let undoable: HashSet<String> = undo.iter().map(|u| u.action_id.clone()).collect();
    record_undo(&run_id, undo).await;
    if !refused.is_empty() {
        release_ids(&run_id, &refused).await;
    }

    // The change log: every action that actually wrote something.
    let at = now_ms();
    let done: HashSet<&String> = results.iter().filter(|r| r.ok).map(|r| &r.action_id).collect();
    let log: Vec<AppliedRecord> = fresh
        .iter()
        .filter(|a| done.contains(&a.id))
        .map(|a| AppliedRecord {
            action_id: a.id.clone(),
            kind: a.kind.clone(),
            path: a.path.clone(),
            canon_path: a.canon_path.clone(),
            params: a.params.clone(),
            applied_at_ms: at,
            undo_available: undoable.contains(&a.id),
        })
        .collect();
    record_applied(&run_id, log).await;

    // Approving one action at a time must keep working: every proposal still
    // pending on a path just written holds the *pre-write* snapshot, and would
    // fail the stale check on its own approval. Re-stamp them with what the
    // server holds now.
    if !rehashed.is_empty() {
        let applied: HashSet<&String> = fresh.iter().map(|a| &a.id).collect();
        let runs = registry.runs.lock().await;
        if let Some(handle) = runs.get(&run_id) {
            let mut pending = handle.proposed.lock().await;
            let done: HashSet<String> = applied.into_iter().cloned().collect();
            apply_rehashes(&mut pending, &done, &rehashed);
        }
    }

    for id in already {
        results.push(ActionApplyResult {
            action_id: id,
            ok: false,
            message: "כבר בוצע".to_string(),
            params: Vec::new(),
            undo: None,
        });
    }
    results.extend(blocked_results);

    for r in &results {
        events::action_applied(&app, &run_id, r);
    }
    sync_history_applied(&app, &run_id).await;
    Ok(results)
}

/// Undo one applied action: write the previous values back.
///
/// Limitation: Yemot's `UpdateExtension` documents no way to *delete* a key, so
/// a key that did not exist before the write is restored as an empty value
/// (`key=`), not removed. A file that did not exist before an `upload_text_file`
/// is not deleted either — file deletion (`FileAction`) is a denied capability.
#[tauri::command]
pub async fn undo_action(
    app: AppHandle,
    run_id: String,
    action_id: String,
) -> Result<ActionApplyResult, String> {
    let (client, record) = {
        let map = write_state().lock().await;
        let state = map
            .get(&run_id)
            .ok_or_else(|| "אין מידע לביטול עבור ריצה זו".to_string())?;
        let record = state
            .undo
            .get(&action_id)
            .cloned()
            .ok_or_else(|| "לא נמצאה פעולה לביטול".to_string())?;
        (state.client.clone(), record)
    };

    // An undo restores the *whole* previous state, so it is only safe while the
    // server still holds exactly what the apply left behind. A later run, the
    // web UI or another session may have changed it since.
    if let Some(expected) = record.post_hash.as_deref() {
        let current = if record.kind == "upload_text_file" {
            client
                .get_text_file(&record.canon_path)
                .await
                .map(|f| yemot::content_hash(f.exists, &f.contents))
        } else {
            client
                .get_ext_ini_fresh(&record.canon_path)
                .await
                .map(|r| r.snapshot_hash())
        };
        match current {
            Ok(now) if now != expected => {
                return Ok(ActionApplyResult {
                    action_id,
                    ok: false,
                    message: UNDO_STALE_MSG.to_string(),
                    params: Vec::new(),
                    undo: None,
                })
            }
            Ok(_) => {}
            Err(_) => {
                return Ok(ActionApplyResult {
                    action_id,
                    ok: false,
                    message: UNVERIFIABLE_MSG.to_string(),
                    params: Vec::new(),
                    undo: None,
                })
            }
        }
    }

    let result = if record.kind == "upload_text_file" {
        match record.contents.clone() {
            Some(previous) => match client.upload_text_file(&record.canon_path, &previous).await {
                Ok(_) => ActionApplyResult {
                    action_id: action_id.clone(),
                    ok: true,
                    message: format!("התוכן הקודם של {} שוחזר", record.path),
                    params: Vec::new(),
                    undo: None,
                },
                Err(e) => ActionApplyResult {
                    action_id: action_id.clone(),
                    ok: false,
                    message: yemot::render_error(&e),
                    params: Vec::new(),
                    undo: None,
                },
            },
            None => ActionApplyResult {
                action_id: action_id.clone(),
                ok: false,
                message: format!(
                    "הקובץ {} לא היה קיים לפני הפעולה, ומחיקת קבצים אינה נתמכת ביישום. מחק אותו ידנית בממשק ימות המשיח.",
                    record.path
                ),
                params: Vec::new(),
                undo: None,
            },
        }
    } else {
        let params: Vec<(String, String)> = record
            .params
            .iter()
            .map(|(k, v)| (k.clone(), v.clone().unwrap_or_default()))
            .collect();
        if params.is_empty() {
            return Err("אין ערכים קודמים לשחזור".to_string());
        }
        let created: Vec<&str> = record
            .params
            .iter()
            .filter(|(_, v)| v.is_none())
            .map(|(k, _)| k.as_str())
            .collect();
        match client.update_extension(&record.canon_path, &params).await {
            Ok(outcomes) => {
                let ok = outcomes.iter().all(|o| o.applied);
                let mut message = yemot::render_outcomes(&record.canon_path, &outcomes);
                if !created.is_empty() {
                    message.push_str(&format!(
                        "הערה: המפתחות {} לא היו קיימים לפני הפעולה ונכתבו כערך ריק — ימות המשיח אינה מאפשרת מחיקת מפתח דרך ה-API.\n",
                        created.join(", ")
                    ));
                }
                ActionApplyResult {
                    action_id: action_id.clone(),
                    ok,
                    message,
                    params: outcomes,
                    undo: None,
                }
            }
            Err(e) => ActionApplyResult {
                action_id: action_id.clone(),
                ok: false,
                message: yemot::render_error(&e),
                params: Vec::new(),
                undo: None,
            },
        }
    };

    if result.ok {
        // An undone action may be approved again.
        release_ids(&run_id, std::slice::from_ref(&action_id)).await;
        let mut map = write_state().lock().await;
        if let Some(state) = map.get_mut(&run_id) {
            state.undo.remove(&action_id);
            // The log row stays — the change happened. It is now marked בוטל,
            // which is also what the continuation status block reports.
            state.undone.insert(action_id.clone());
        }
        drop(map);
        sync_history_applied(&app, &run_id).await;
    }
    Ok(result)
}

/// Undo values for one `set_extension_params` action, taken from the diff that
/// was shown to the user (`before = None` means the key was absent).
fn undo_params_of(a: &ProposedAction) -> Vec<(String, Option<String>)> {
    a.params
        .iter()
        .map(|p| {
            let before = a
                .diff
                .iter()
                .find(|d| d.key == p.key)
                .and_then(|d| d.before.clone());
            (p.key.clone(), before)
        })
        .collect()
}

/// What one `apply_actions` call produced.
struct ApplyOutcome {
    results: Vec<ActionApplyResult>,
    /// Undo records of what actually got written.
    undo: Vec<UndoRecord>,
    /// Ids refused before touching the server, so they can be approved again.
    refused: Vec<String>,
    /// `(kind, canon_path, fresh hash)` for every path this call wrote — the
    /// new baseline for proposals on that path that are still pending.
    rehashed: Vec<(String, String, String)>,
}

/// Re-stamp the still-pending proposals on a path that was just written.
/// Actions in `done` were part of this apply and keep their own record.
fn apply_rehashes(
    pending: &mut [ProposedAction],
    done: &HashSet<String>,
    rehashed: &[(String, String, String)],
) {
    for a in pending.iter_mut() {
        if done.contains(&a.id) {
            continue;
        }
        if let Some((_, _, hash)) = rehashed
            .iter()
            .find(|(kind, path, _)| kind == &a.kind && path == &a.canon_path)
        {
            a.snapshot_hash = Some(hash.clone());
        }
    }
}

/// Group by extension so each path costs exactly one `UpdateExtension`.
async fn apply_actions(
    client: &YemotClient,
    actions: &[ProposedAction],
    audio: &HashMap<String, Attachment>,
) -> ApplyOutcome {
    // Preserve first-seen path order; last value wins per key.
    let mut order: Vec<String> = Vec::new();
    let mut grouped: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for a in actions.iter().filter(|a| a.kind == "set_extension_params") {
        let entry = grouped.entry(a.canon_path.clone()).or_insert_with(|| {
            order.push(a.canon_path.clone());
            Vec::new()
        });
        for p in &a.params {
            if let Some(slot) = entry.iter_mut().find(|(k, _)| k == &p.key) {
                slot.1 = p.value.clone();
            } else {
                entry.push((p.key.clone(), p.value.clone()));
            }
        }
    }

    // Stale-state check: the proposal was built against a snapshot of ext.ini.
    // If the file changed on the server since (another session, the web UI, an
    // earlier approval), the merge the user approved is no longer the merge
    // that would happen — refuse instead of writing over the change.
    //
    // The check fails CLOSED: a read that errors leaves the current state
    // unknown, and writing blind is exactly what this guard exists to prevent.
    let mut stale: HashSet<String> = HashSet::new();
    let mut unverifiable: HashSet<String> = HashSet::new();
    for path in &order {
        let expected: Vec<&String> = actions
            .iter()
            .filter(|a| &a.canon_path == path && a.kind == "set_extension_params")
            .filter_map(|a| a.snapshot_hash.as_ref())
            .collect();
        if expected.is_empty() {
            continue;
        }
        match client.get_ext_ini_fresh(path).await {
            Ok(now) => {
                let current = now.snapshot_hash();
                if expected.iter().any(|h| **h != current) {
                    stale.insert(path.clone());
                }
            }
            Err(_) => {
                unverifiable.insert(path.clone());
            }
        }
    }

    let mut per_path: HashMap<String, Result<Vec<ParamOutcome>, String>> = HashMap::new();
    let mut rehashed: Vec<(String, String, String)> = Vec::new();
    // The state each written path was left in — the baseline an undo compares
    // against.
    let mut post_hash: HashMap<String, String> = HashMap::new();
    for path in &order {
        if stale.contains(path) || unverifiable.contains(path) {
            continue;
        }
        let params = grouped.get(path).cloned().unwrap_or_default();
        let res = client
            .update_extension(path, &params)
            .await
            .map_err(|e| yemot::render_error(&e));
        let wrote = res.as_ref().map(|o| o.iter().any(|p| p.applied)).unwrap_or(false);
        per_path.insert(path.clone(), res);
        if wrote {
            if let Ok(now) = client.get_ext_ini_fresh(path).await {
                let h = now.snapshot_hash();
                post_hash.insert(path.clone(), h.clone());
                rehashed.push(("set_extension_params".to_string(), path.clone(), h));
            }
        }
    }

    let mut out = Vec::new();
    let mut undo = Vec::new();
    let mut refused = Vec::new();
    for a in actions {
        if a.kind == "upload_audio_file" {
            // The bytes are read HERE, not when the action was proposed: the
            // approval dialog may sit open for minutes, and holding a 25 MB
            // buffer per proposal (or uploading a file the user has since
            // replaced) would both be wrong.
            let Some(att) = audio.get(&a.id) else {
                refused.push(a.id.clone());
                out.push(refusal(a, "הקובץ המצורף אינו זמין לריצה זו"));
                continue;
            };
            // Re-measure before reading: the run-start check is minutes old.
            if let Err(msg) = recheck_attachment(att) {
                refused.push(a.id.clone());
                out.push(refusal(a, &msg));
                continue;
            }
            // The overwrite warning was decided at propose time too. If the
            // slot filled up since, the user approved a "new file" that would
            // now silently destroy a recording. Fails closed, like every other
            // stale guard here.
            if !a.exists {
                let (dir, name) = tools::dir_and_name(&a.canon_path);
                match client.list_files(&dir).await {
                    Ok(files) => {
                        let present: Vec<String> = files.into_iter().map(|f| f.name).collect();
                        if audio_slot_taken(a.exists, &present, &name) {
                            refused.push(a.id.clone());
                            out.push(refusal(a, AUDIO_APPEARED_MSG));
                            continue;
                        }
                    }
                    Err(e) if yemot::is_missing_file(&e) => {
                        // Proposed for an extension the same task creates; the
                        // creation was not applied (or failed), so the upload
                        // has nowhere to go.
                        refused.push(a.id.clone());
                        out.push(refusal(a, AUDIO_FOLDER_MISSING_MSG));
                        continue;
                    }
                    Err(_) => {
                        refused.push(a.id.clone());
                        out.push(refusal(a, UNVERIFIABLE_MSG));
                        continue;
                    }
                }
            }
            let bytes = match std::fs::read(&att.local_path) {
                Ok(b) => b,
                Err(_) => {
                    refused.push(a.id.clone());
                    out.push(refusal(
                        a,
                        &format!("לא ניתן לקרוא את הקובץ {} מהמחשב", att.name),
                    ));
                    continue;
                }
            };
            match client.upload_audio_file(&a.canon_path, bytes, &att.name).await {
                // No undo record on purpose: Yemot exposes no file delete
                // (`file_action` is a denied capability) and the previous audio
                // is not recoverable, so an "undo" here would be a lie.
                Ok(size) => out.push(ActionApplyResult {
                    action_id: a.id.clone(),
                    ok: true,
                    message: format!("הקובץ {} הועלה אל {} ({} בתים)", att.name, a.path, size),
                    params: Vec::new(),
                    undo: None,
                }),
                Err(e) => {
                    refused.push(a.id.clone());
                    out.push(ActionApplyResult {
                        action_id: a.id.clone(),
                        ok: false,
                        message: yemot::render_error(&e),
                        params: Vec::new(),
                        undo: None,
                    });
                }
            }
            continue;
        }

        if a.kind == "upload_text_file" {
            // The same stale guard the extension writes get. `upload_text_file`
            // replaces the whole file, so a change since the proposal would be
            // lost silently.
            if let Some(expected) = a.snapshot_hash.as_deref() {
                match client.get_text_file(&a.canon_path).await {
                    Ok(f) if yemot::content_hash(f.exists, &f.contents) != expected => {
                        refused.push(a.id.clone());
                        out.push(refusal(a, STALE_MSG));
                        continue;
                    }
                    Ok(_) => {}
                    Err(_) => {
                        refused.push(a.id.clone());
                        out.push(refusal(a, UNVERIFIABLE_MSG));
                        continue;
                    }
                }
            }
            let contents = a.contents.clone().unwrap_or_default();
            let r = client.upload_text_file(&a.canon_path, &contents).await;
            out.push(match r {
                Ok(previous) => {
                    let after = yemot::content_hash(true, &contents);
                    rehashed.push((a.kind.clone(), a.canon_path.clone(), after.clone()));
                    let record = UndoRecord {
                        action_id: a.id.clone(),
                        kind: a.kind.clone(),
                        path: a.path.clone(),
                        canon_path: a.canon_path.clone(),
                        params: Vec::new(),
                        // What the server had a moment ago beats what the
                        // proposal saw; fall back to the proposal's snapshot.
                        contents: previous.or_else(|| a.previous.clone()),
                        post_hash: Some(after),
                    };
                    let undo_json = serde_json::to_value(&record).ok();
                    undo.push(record);
                    ActionApplyResult {
                        action_id: a.id.clone(),
                        ok: true,
                        message: format!("הקובץ {} נכתב", a.path),
                        params: Vec::new(),
                        undo: undo_json,
                    }
                }
                Err(e) => {
                    refused.push(a.id.clone());
                    ActionApplyResult {
                        action_id: a.id.clone(),
                        ok: false,
                        message: yemot::render_error(&e),
                        params: Vec::new(),
                        undo: None,
                    }
                }
            });
            continue;
        }

        if unverifiable.contains(&a.canon_path) {
            refused.push(a.id.clone());
            out.push(refusal(a, UNVERIFIABLE_MSG));
            continue;
        }

        if stale.contains(&a.canon_path) {
            refused.push(a.id.clone());
            out.push(refusal(a, STALE_MSG));
            continue;
        }

        match per_path.get(&a.canon_path) {
            Some(Ok(outcomes)) => {
                let mine: Vec<ParamOutcome> = a
                    .params
                    .iter()
                    .filter_map(|p| outcomes.iter().find(|o| o.key == p.key).cloned())
                    .collect();
                let ok = !mine.is_empty() && mine.iter().all(|o| o.applied);
                let mut undo_json = None;
                if ok {
                    let record = UndoRecord {
                        action_id: a.id.clone(),
                        kind: a.kind.clone(),
                        path: a.path.clone(),
                        canon_path: a.canon_path.clone(),
                        params: undo_params_of(a),
                        contents: None,
                        post_hash: post_hash.get(&a.canon_path).cloned(),
                    };
                    undo_json = serde_json::to_value(&record).ok();
                    undo.push(record);
                } else {
                    refused.push(a.id.clone());
                }
                out.push(ActionApplyResult {
                    action_id: a.id.clone(),
                    ok,
                    message: yemot::render_outcomes(&a.canon_path, &mine),
                    params: mine,
                    undo: undo_json,
                });
            }
            Some(Err(msg)) => {
                refused.push(a.id.clone());
                out.push(ActionApplyResult {
                    action_id: a.id.clone(),
                    ok: false,
                    message: msg.clone(),
                    params: Vec::new(),
                    undo: None,
                });
            }
            None => {
                refused.push(a.id.clone());
                out.push(refusal(a, "הפעולה לא נמצאה"));
            }
        }
    }
    ApplyOutcome {
        results: out,
        undo,
        refused,
        rehashed,
    }
}

/// A "nothing was written" result for one action.
fn refusal(a: &ProposedAction, message: &str) -> ActionApplyResult {
    ActionApplyResult {
        action_id: a.id.clone(),
        ok: false,
        message: message.to_string(),
        params: Vec::new(),
        undo: None,
    }
}

// ---------------------------------------------------------------------------
// The loop
// ---------------------------------------------------------------------------

struct RunContext {
    app: AppHandle,
    run_id: String,
    /// The chain this run belongs to (the `run_id` of its first run).
    task_id: String,
    provider_name: String,
    model: String,
    auto_apply: bool,
    client: Arc<YemotClient>,
    proposed: Arc<Mutex<Vec<ProposedAction>>>,
    cancel: CancellationToken,
    /// Where the loop leaves its messages for a later continuation.
    transcript: Arc<Mutex<Vec<Message>>>,
    audio: Arc<Mutex<HashMap<String, Attachment>>>,
    attachments: Arc<Vec<Attachment>>,
    /// Instruction chain + chain-wide usage totals, shared with the handle.
    chain: Arc<Mutex<ChainState>>,
    save_history: bool,
}

/// One agent run over `messages`. A fresh run passes a single user message; a
/// continuation passes the parent's transcript plus one more — from here on the
/// two are the same run.
async fn run_loop(ctx: RunContext, provider: Box<dyn Provider>, messages: Vec<Message>) {
    let started_at = Instant::now();
    events::started(&ctx.app, &ctx.run_id, &ctx.provider_name, &ctx.model);

    let tools = tools::tool_specs();
    let system = prompt::system_blocks(&ctx.provider_name);

    let mut messages: Vec<Message> = messages;

    let tool_ctx = ToolCtx {
        app: ctx.app.clone(),
        run_id: ctx.run_id.clone(),
        client: ctx.client.clone(),
        auto_apply: ctx.auto_apply,
        reads: Arc::new(Mutex::new(HashSet::new())),
        ext_reads: Arc::new(Mutex::new(HashMap::new())),
        proposed: ctx.proposed.clone(),
        action_seq: Arc::new(AtomicUsize::new(0)),
        session_error: Arc::new(Mutex::new(None)),
        attachments: ctx.attachments.clone(),
        audio: ctx.audio.clone(),
    };

    let mut usage = Usage::default();
    let mut turns_done: u32 = 0;
    let mut tool_calls: u32 = 0;
    let mut mutating_calls: u32 = 0;
    let mut final_text = String::new();
    let mut stop = "max_turns";
    let mut ok = true;
    let mut guard = LoopGuard::default();

    for turn in 1..=MAX_TURNS {
        if ctx.cancel.is_cancelled() {
            stop = "cancelled";
            ok = false;
            break;
        }
        if started_at.elapsed() > RUN_BUDGET {
            stop = "max_turns";
            break;
        }
        turns_done = turn;

        // Context hygiene, before the prompt is assembled. Nothing is rewritten
        // — an over-long conversation is stopped, not silently truncated.
        if estimate_context(&system, &messages) > CONTEXT_HARD_LIMIT {
            events::error(
                &ctx.app,
                &ctx.run_id,
                "internal",
                "השיחה ארוכה מדי להמשך. פצל את הבקשה למשימות קטנות יותר.",
            );
            stop = "truncated";
            ok = false;
            break;
        }

        events::turn_start(&ctx.app, &ctx.run_id, turn, MAX_TURNS);

        let req = ProviderRequest {
            model: ctx.model.clone(),
            system: system.clone(),
            messages: messages.clone(),
            tools: tools.clone(),
            turn,
        };

        let response = match call_with_retry(&ctx, provider.as_ref(), &req, started_at).await {
            Ok(r) => r,
            Err(ProviderError::Cancelled) => {
                stop = "cancelled";
                ok = false;
                break;
            }
            Err(ProviderError::Refusal) => {
                stop = "refusal";
                ok = false;
                break;
            }
            Err(e) => {
                events::error(&ctx.app, &ctx.run_id, e.code(), &e.message());
                stop = "error";
                ok = false;
                break;
            }
        };
        usage.add(&response.usage);

        let text = response.text();
        if !text.trim().is_empty() {
            final_text = text.clone();
            events::assistant_text(&ctx.app, &ctx.run_id, turn, &text);
        }
        if !response.content.is_empty() {
            messages.push(Message {
                role: Role::Assistant,
                content: response.content.clone(),
            });
        }

        // Tool calls are read out BEFORE the stop reason is consulted: several
        // providers report "stop"/"STOP" alongside a function call.
        let calls: Vec<(String, String, Value)> = response
            .tool_uses()
            .into_iter()
            .map(|(id, name, input)| (id.to_string(), name.to_string(), input.clone()))
            .collect();

        if calls.is_empty() {
            stop = match response.stop {
                StopReason::MaxTokens => "truncated",
                StopReason::Refusal => "refusal",
                _ => "end_turn",
            };
            ok = stop == "end_turn";
            break;
        }

        tool_calls += calls.len().min(MAX_TOOL_CALLS_PER_TURN) as u32;
        mutating_calls += calls
            .iter()
            .take(MAX_TOOL_CALLS_PER_TURN)
            .filter(|(_, n, _)| tools::is_mutating(n))
            .count() as u32;

        let results = run_tools(&ctx, &tool_ctx, &calls, &mut guard).await;
        messages.push(Message {
            role: Role::User,
            content: results,
        });

        if guard.exhausted() {
            events::error(
                &ctx.app,
                &ctx.run_id,
                "internal",
                &format!(
                    "המודל חזר על אותן קריאות שוב ושוב ({} חסימות). נסח את הבקשה מחדש או פצל אותה.",
                    guard.blocked()
                ),
            );
            stop = "error";
            ok = false;
            break;
        }

        // A dead session cannot be recovered inside the loop.
        if let Some(e) = tool_ctx.session_error.lock().await.clone() {
            events::error(
                &ctx.app,
                &ctx.run_id,
                "session_expired",
                &yemot::render_error(&e),
            );
            stop = "error";
            ok = false;
            break;
        }
    }

    // Legacy safety net: a model that "described" the change instead of calling
    // the write tool still produces actionable rows.
    if mutating_calls == 0 && ctx.proposed.lock().await.is_empty() {
        let legacy = legacy_actions_from_text(&final_text);
        if !legacy.is_empty() {
            let mut slot = ctx.proposed.lock().await;
            for a in legacy {
                events::action_proposed(&ctx.app, &ctx.run_id, &a);
                slot.push(a);
            }
        }
    }

    let actions = ctx.proposed.lock().await.clone();
    events::actions_proposed(&ctx.app, &ctx.run_id, &actions);

    // Retained for `continue_agent_run`; dropped with the handle when the next
    // run claims a slot.
    *ctx.transcript.lock().await = messages;

    let cost = pricing::cost_usd(&ctx.model, &usage);

    // The local task record — written for every run with a transcript, ok or
    // not, cancelled included, so the task can be resumed after a restart.
    save_task_record(
        &ctx,
        RunOutcome {
            actions: &actions,
            usage: &usage,
            turns: turns_done,
            tool_calls,
            cost,
            ok,
            stop,
            final_text: &final_text,
        },
    )
    .await;

    let run_usage = RunUsage {
        input_tokens: usage.input,
        output_tokens: usage.output,
        cache_read_tokens: usage.cache_read,
        cache_write_tokens: usage.cache_write,
        turns: turns_done,
        tool_calls,
        elapsed_ms: started_at.elapsed().as_millis() as u64,
        cache_hit_pct: usage.cache_hit_pct(),
        cost_usd: cost,
        price_list_date: pricing::price_list_date(cost),
    };
    events::finished(
        &ctx.app,
        &ctx.run_id,
        &ctx.task_id,
        ok,
        stop,
        &final_text,
        &run_usage,
    );
}

/// What one finished run contributes to its task record.
struct RunOutcome<'a> {
    actions: &'a [ProposedAction],
    usage: &'a Usage,
    turns: u32,
    tool_calls: u32,
    cost: Option<f64>,
    ok: bool,
    stop: &'a str,
    final_text: &'a str,
}

/// Rewrite this task's record with the newest run of the chain.
///
/// Never fails the run: every error is logged inside `history` and swallowed.
/// The file write itself happens on the blocking pool — the records are small,
/// but a slow disk must not sit in the middle of the run's last await.
async fn save_task_record(ctx: &RunContext, out: RunOutcome<'_>) {
    let transcript = ctx.transcript.lock().await.clone();
    if !history::should_save(ctx.save_history, &transcript) {
        return;
    }
    // Approval happens *after* the run ends, so this is normally empty on a
    // fresh save; `approve_actions` / `undo_action` refresh it afterwards.
    let applied = {
        let map = write_state().lock().await;
        match map.get(&ctx.run_id) {
            Some(s) => history::applied_ids(&s.applied, &s.undone),
            None => Vec::new(),
        }
    };
    let chain = {
        let mut c = ctx.chain.lock().await;
        c.usage.add_run(out.usage, out.turns, out.tool_calls, out.cost);
        c.clone()
    };
    let record = TaskRecord {
        task_id: chain.task_id,
        last_run_id: ctx.run_id.clone(),
        created_at_ms: chain.created_at_ms,
        updated_at_ms: history::now_ms(),
        provider: ctx.provider_name.clone(),
        model: ctx.model.clone(),
        instructions: chain.instructions,
        final_text: out.final_text.to_string(),
        ok: out.ok,
        stop: out.stop.to_string(),
        transcript,
        attachments: ctx.attachments.as_ref().clone(),
        actions: out.actions.to_vec(),
        applied_action_ids: applied,
        usage: chain.usage,
    };
    history::save_task(&ctx.app, ctx.save_history, record).await;
}

/// Push the run's applied-ids into its saved task record, so a task resumed
/// from history reports בוצע / לא אושר instead of an empty status block.
async fn sync_history_applied(app: &AppHandle, run_id: &str) {
    let applied = {
        let map = write_state().lock().await;
        match map.get(run_id) {
            Some(s) => history::applied_ids(&s.applied, &s.undone),
            None => return,
        }
    };
    history::sync_applied(app, run_id, applied).await;
}

/// One turn's tools: read-only ones together, mutating ones one at a time.
/// Every result comes back in ONE user message, in call order.
async fn run_tools(
    ctx: &RunContext,
    tool_ctx: &ToolCtx,
    calls: &[(String, String, Value)],
    guard: &mut LoopGuard,
) -> Vec<ContentBlock> {
    let mut results: Vec<Option<ToolOutcome>> = vec![None; calls.len()];

    // Over-calling is a model bug, not a reason to fail the run.
    for (i, (_, name, _)) in calls.iter().enumerate().skip(MAX_TOOL_CALLS_PER_TURN) {
        results[i] = Some(ToolOutcome {
            content: format!(
                "TOO_MANY_TOOL_CALLS: מותר עד {} קריאות בתור אחד. {} לא בוצע — קרא לו בתור הבא.",
                MAX_TOOL_CALLS_PER_TURN, name
            ),
            is_error: true,
        });
    }
    let live = &calls[..calls.len().min(MAX_TOOL_CALLS_PER_TURN)];

    for (id, name, input) in live {
        events::tool_started(&ctx.app, &ctx.run_id, id, name, &tools::label_for(name, input));
    }

    // Loop detection runs before dispatch: a blocked call costs nothing and
    // still gets a tool result, so the conversation stays well-formed.
    for (i, (id, name, input)) in live.iter().enumerate() {
        if guard.check(name, input) {
            let out = ToolOutcome { content: LOOP_MSG.to_string(), is_error: true };
            events::tool_finished(&ctx.app, &ctx.run_id, id, false, &out.summary(), 0);
            results[i] = Some(out);
        }
    }

    // Reads: in parallel — knowledge lookups and extension reads are independent.
    let read_idx: Vec<usize> = live
        .iter()
        .enumerate()
        .filter(|(i, (_, n, _))| results[*i].is_none() && !tools::is_mutating(n))
        .map(|(i, _)| i)
        .collect();
    let futures = read_idx.iter().map(|&i| {
        let (id, name, input) = &live[i];
        async move {
            let t0 = Instant::now();
            let out = tools::execute_tool(name, input, id, tool_ctx).await;
            (i, out, t0.elapsed().as_millis() as u64)
        }
    });
    for (i, out, ms) in join_all(futures).await {
        let (id, _, _) = &live[i];
        events::tool_finished(&ctx.app, &ctx.run_id, id, !out.is_error, &out.summary(), ms);
        results[i] = Some(out);
    }

    // Writes: sequential, and cancellation is honoured between them (never
    // inside one — an interrupted Yemot write is worse than a slow cancel).
    for (i, (id, name, input)) in live.iter().enumerate() {
        if !tools::is_mutating(name) || results[i].is_some() {
            continue;
        }
        if ctx.cancel.is_cancelled() {
            results[i] = Some(ToolOutcome {
                content: "CANCELLED: הריצה בוטלה לפני ביצוע הפעולה.".to_string(),
                is_error: true,
            });
            continue;
        }
        let t0 = Instant::now();
        let out = tools::execute_tool(name, input, id, tool_ctx).await;
        events::tool_finished(
            &ctx.app,
            &ctx.run_id,
            id,
            !out.is_error,
            &out.summary(),
            t0.elapsed().as_millis() as u64,
        );
        results[i] = Some(out);
    }

    calls
        .iter()
        .zip(results)
        .map(|((id, name, _), out)| {
            let out = out.unwrap_or_else(|| ToolOutcome {
                content: "INTERNAL: הכלי לא הופעל.".to_string(),
                is_error: true,
            });
            ContentBlock::ToolResult {
                tool_use_id: id.clone(),
                name: name.clone(),
                content: out.content,
                is_error: out.is_error,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Retry
// ---------------------------------------------------------------------------

/// ±25% jitter without pulling in `rand` — the clock is entropy enough here.
fn jitter(base_ms: u64) -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    let spread = base_ms / 2; // full width = 50% of base
    let offset = nanos % spread.max(1);
    base_ms - spread / 2 + offset
}

pub fn backoff_ms(attempt: u32, retry_after: Option<u64>) -> u64 {
    if let Some(s) = retry_after {
        return (s * 1000).min(60_000);
    }
    jitter(1000u64 << (attempt.saturating_sub(1)).min(3))
}

/// Backoff sleeps are part of the run budget: waiting 8s for a retry that can
/// only start after the deadline burns the user's time for nothing.
pub fn budget_allows_retry(elapsed: Duration, wait_ms: u64) -> bool {
    elapsed + Duration::from_millis(wait_ms) <= RUN_BUDGET
}

async fn call_with_retry(
    ctx: &RunContext,
    provider: &dyn Provider,
    req: &ProviderRequest,
    started_at: Instant,
) -> Result<ProviderResponse, ProviderError> {
    let mut last: ProviderError = ProviderError::Transient("—".to_string());
    // The provider streams into this; every batch becomes one `agent:text_delta`.
    let on_delta = |d: &str| events::text_delta(&ctx.app, &ctx.run_id, d);
    for attempt in 1..=MAX_ATTEMPTS {
        if attempt > 1 {
            // Whatever the failed attempt streamed is not part of this turn.
            events::text_delta_reset(&ctx.app, &ctx.run_id);
        }
        match provider.complete(req, &ctx.cancel, &on_delta).await {
            Ok(r) => return Ok(r),
            // Auth / BadRequest / Refusal / Cancelled: retrying cannot help.
            Err(e) if !e.retryable() => return Err(e),
            Err(e) => {
                if attempt == MAX_ATTEMPTS {
                    return Err(e);
                }
                let retry_after = match &e {
                    ProviderError::RateLimited { retry_after } => *retry_after,
                    _ => None,
                };
                let wait = backoff_ms(attempt, retry_after);
                if !budget_allows_retry(started_at.elapsed(), wait) {
                    return Err(e);
                }
                events::retry(
                    &ctx.app,
                    &ctx.run_id,
                    attempt,
                    MAX_ATTEMPTS,
                    &e.message(),
                    wait,
                );
                last = e;
                tokio::select! {
                    _ = ctx.cancel.cancelled() => return Err(ProviderError::Cancelled),
                    _ = tokio::time::sleep(Duration::from_millis(wait)) => {}
                }
            }
        }
    }
    Err(last)
}

// ---------------------------------------------------------------------------
// `[מצב נוכחי]`
// ---------------------------------------------------------------------------

async fn root_tree_block(client: &YemotClient) -> Option<String> {
    let nodes = client.list_extensions("/", 1).await.ok()?;
    Some(render_tree_block(&yemot::render_tree(&nodes), nodes.len()))
}

/// ≤ `TREE_LINES` lines, with a count of what was left out.
pub fn render_tree_block(rendered: &str, total: usize) -> String {
    let lines: Vec<&str> = rendered.lines().filter(|l| !l.trim().is_empty()).collect();
    let mut out = String::from("[מצב נוכחי]\n");
    for l in lines.iter().take(TREE_LINES) {
        out.push_str(l);
        out.push('\n');
    }
    if total > TREE_LINES {
        out.push_str(&format!("… ועוד {} שלוחות\n", total - TREE_LINES));
    }
    out
}

// ---------------------------------------------------------------------------
// Legacy `parseActionList` (port of +page.svelte)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LegacyAction {
    pub path: String,
    pub key: String,
    pub value: String,
    pub description: String,
}

/// Rust port of the frontend's `parseActionList`: a JSON array of actions, or
/// Yemot API URL lines preceded by `הסבר:` lines. Rows without a `path` are
/// dropped — unlike the JS fallback, which produced un-executable rows.
pub fn parse_action_list(raw: &str) -> Vec<LegacyAction> {
    if let Ok(Value::Array(items)) = serde_json::from_str::<Value>(raw.trim()) {
        let out: Vec<LegacyAction> = items
            .iter()
            .filter_map(|i| {
                let path = i.get("path")?.as_str()?.trim().to_string();
                if path.is_empty() {
                    return None;
                }
                Some(LegacyAction {
                    path,
                    key: i.get("key").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    value: i.get("value").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    description: i
                        .get("description")
                        .or_else(|| i.get("desc"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                })
            })
            .collect();
        if !out.is_empty() {
            return out;
        }
    }

    let mut actions = Vec::new();
    let mut description = String::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("הסבר:") {
            description = rest.trim().to_string();
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if (lower.starts_with("http://") || lower.starts_with("https://")) && trimmed.contains('?') {
            if let Ok(url) = reqwest::Url::parse(trimmed) {
                let path = url
                    .query_pairs()
                    .find(|(k, _)| k == "path")
                    .map(|(_, v)| v.to_string())
                    .unwrap_or_default();
                if path.is_empty() {
                    continue;
                }
                for (k, v) in url.query_pairs() {
                    if k == "path" || k == "token" {
                        continue;
                    }
                    actions.push(LegacyAction {
                        path: path.clone(),
                        key: k.to_string(),
                        value: v.to_string(),
                        description: description.clone(),
                    });
                }
                description.clear();
            }
        }
    }
    actions
}

/// Legacy rows → contract actions. No diff and no `preview_merge`: nothing was
/// read, so `risk` stays `low` and the UI shows the raw key/value rows.
pub fn legacy_actions_from_text(text: &str) -> Vec<ProposedAction> {
    let rows = parse_action_list(text);
    let mut order: Vec<String> = Vec::new();
    let mut by_path: HashMap<String, (Vec<ActionParam>, String)> = HashMap::new();
    for r in rows {
        let canon = yemot::canon_ext(&r.path).unwrap_or_else(|_| r.path.clone());
        let entry = by_path.entry(canon.clone()).or_insert_with(|| {
            order.push(canon.clone());
            (Vec::new(), r.description.clone())
        });
        if entry.1.is_empty() {
            entry.1 = r.description.clone();
        }
        entry.0.push(ActionParam { key: r.key, value: r.value });
    }
    order
        .into_iter()
        .enumerate()
        .filter_map(|(i, canon)| {
            let (params, reason) = by_path.remove(&canon)?;
            Some(ProposedAction {
                id: format!("a_legacy_{}", i + 1),
                tool_use_id: String::new(),
                kind: "set_extension_params".to_string(),
                path: yemot::display_path(&canon),
                canon_path: canon,
                params,
                contents: None,
                reason,
                risk: "low".to_string(),
                exists: false,
                diff: Vec::new(),
                warnings: vec!["נוצר מטקסט חופשי של המודל, ללא בדיקת מצב קיים".to_string()],
                previous: None,
                snapshot_hash: None,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_is_1_2_4_8_seconds_with_jitter() {
        for (attempt, base) in [(1u32, 1000u64), (2, 2000), (3, 4000), (4, 8000)] {
            let ms = backoff_ms(attempt, None);
            assert!(
                ms >= base * 3 / 4 && ms <= base * 5 / 4,
                "attempt {} gave {}ms",
                attempt,
                ms
            );
        }
    }

    #[test]
    fn retry_after_wins_and_is_bounded() {
        assert_eq!(backoff_ms(1, Some(7)), 7000);
        assert_eq!(backoff_ms(1, Some(9999)), 60_000);
    }

    #[test]
    fn tree_block_is_capped_at_twenty_lines() {
        let rendered: String = (1..=30).map(|i| format!("/{}  type=menu  title=-\n", i)).collect();
        let block = render_tree_block(&rendered, 30);
        assert!(block.starts_with("[מצב נוכחי]\n"));
        assert_eq!(block.lines().count(), 1 + 20 + 1);
        assert!(block.contains("… ועוד 10 שלוחות"));

        let small: String = (1..=3).map(|i| format!("/{}  type=menu  title=-\n", i)).collect();
        let block = render_tree_block(&small, 3);
        assert!(!block.contains("ועוד"));
    }

    #[test]
    fn legacy_url_lines_are_parsed() {
        let raw = "כדי להגדיר:\n\
                   הסבר: הגדרת שלוחה 3 כתפריט\n\
                   https://call2all.co.il/ym/api/UpdateExtension?token=SECRET&path=ivr2:/3&type=menu&title=ראשי\n\
                   סיימתי.";
        let a = parse_action_list(raw);
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].path, "ivr2:/3");
        assert_eq!(a[0].key, "type");
        assert_eq!(a[0].value, "menu");
        assert_eq!(a[0].description, "הגדרת שלוחה 3 כתפריט");
        assert_eq!(a[1].key, "title");
        // the token is never turned into an action
        assert!(a.iter().all(|x| x.key != "token"));
    }

    #[test]
    fn legacy_json_array_is_parsed() {
        let raw = r#"[{"path":"/4","key":"type","value":"menu","desc":"תפריט"}]"#;
        let a = parse_action_list(raw);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].description, "תפריט");
    }

    #[test]
    fn plain_prose_yields_nothing() {
        assert!(parse_action_list("סיימתי להגדיר, type=menu הוגדר בשלוחה.").is_empty());
        assert!(parse_action_list("").is_empty());
    }

    #[test]
    fn legacy_rows_group_into_one_action_per_path() {
        let raw = "הסבר: תפריט ראשי\n\
                   https://call2all.co.il/ym/api/UpdateExtension?path=ivr2:/3&type=menu&title=x\n\
                   https://call2all.co.il/ym/api/UpdateExtension?path=ivr2:/4&type=playfile\n";
        let actions = legacy_actions_from_text(raw);
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].id, "a_legacy_1");
        assert_eq!(actions[0].path, "/3");
        assert_eq!(actions[0].params.len(), 2);
        assert_eq!(actions[0].reason, "תפריט ראשי");
        assert_eq!(actions[0].risk, "low");
        assert!(actions[0].diff.is_empty());
        assert_eq!(actions[1].path, "/4");
    }

    // -- loop detection ----------------------------------------------------

    #[test]
    fn third_identical_call_is_blocked() {
        let mut g = LoopGuard::default();
        let input = serde_json::json!({"path": "/3"});
        assert!(!g.check("get_extension_config", &input));
        assert!(!g.check("get_extension_config", &input));
        assert!(g.check("get_extension_config", &input));
        assert_eq!(g.blocked(), 1);
        assert!(!g.exhausted());
    }

    #[test]
    fn paths_are_canonicalized_before_comparison() {
        assert_eq!(
            LoopGuard::signature("get_extension_config", &serde_json::json!({"path":"/3"})),
            LoopGuard::signature("get_extension_config", &serde_json::json!({"path":"ivr2:/3"}))
        );
        // different arguments are different calls
        assert_ne!(
            LoopGuard::signature("get_extension_config", &serde_json::json!({"path":"/3"})),
            LoopGuard::signature("get_extension_config", &serde_json::json!({"path":"/4"}))
        );
        // key order in the JSON must not matter
        assert_eq!(
            LoopGuard::signature("x", &serde_json::json!({"a":1,"b":2})),
            LoopGuard::signature("x", &serde_json::json!({"b":2,"a":1}))
        );
    }

    #[test]
    fn different_calls_do_not_share_a_budget() {
        let mut g = LoopGuard::default();
        for i in 0..10 {
            assert!(!g.check("search_knowledge", &serde_json::json!({ "query": i.to_string() })));
        }
        assert_eq!(g.blocked(), 0);
    }

    #[test]
    fn five_blocked_repeats_exhaust_the_run() {
        let mut g = LoopGuard::default();
        let input = serde_json::json!({"query": "תפריט"});
        for _ in 0..7 {
            g.check("search_knowledge", &input);
        }
        assert_eq!(g.blocked(), 5);
        assert!(g.exhausted());
        assert!(LOOP_MSG.starts_with("LOOP_DETECTED:"));
    }

    // -- context management ------------------------------------------------

    fn tool_pair(id: &str, name: &str, body: &str) -> [Message; 2] {
        [
            Message {
                role: Role::Assistant,
                content: vec![ContentBlock::ToolUse {
                    id: id.into(),
                    name: name.into(),
                    input: serde_json::json!({}),
                }],
            },
            Message {
                role: Role::User,
                content: vec![ContentBlock::ToolResult {
                    tool_use_id: id.into(),
                    name: name.into(),
                    content: body.into(),
                    is_error: false,
                }],
            },
        ]
    }

    #[test]
    fn context_estimate_grows_with_content() {
        let system = vec!["a".repeat(400)];
        let small = vec![Message::user_text("שלום")];
        let big = {
            let mut v = vec![Message::user_text("שלום")];
            v.extend(tool_pair("t1", "search_knowledge", &"x".repeat(20_000)));
            v
        };
        assert!(estimate_context(&system, &big) > estimate_context(&system, &small));
        assert!(estimate_context(&system, &small) > 0);
    }

    #[test]
    fn only_the_hard_limit_bounds_the_context() {
        // The single context lever: a long conversation is stopped, never
        // rewritten — rewriting would invalidate the cached prefix.
        assert_eq!(CONTEXT_HARD_LIMIT, 150_000);
        let mut messages = vec![Message::user_text("בקשה")];
        for i in 0..6 {
            messages.extend(tool_pair(&format!("t{}", i), "search_knowledge", &"y".repeat(500)));
        }
        assert!(estimate_context(&[], &messages) < CONTEXT_HARD_LIMIT);
    }

    // -- write safety ------------------------------------------------------

    fn action_with_diff() -> ProposedAction {
        ProposedAction {
            id: "a_1".into(),
            tool_use_id: "toolu_1".into(),
            kind: "set_extension_params".into(),
            path: "/3".into(),
            canon_path: "ivr2:/3".into(),
            params: vec![
                ActionParam { key: "type".into(), value: "menu".into() },
                ActionParam { key: "title".into(), value: "חדש".into() },
            ],
            contents: None,
            reason: "בדיקה".into(),
            risk: "overwrite".into(),
            exists: true,
            diff: vec![
                super::events::DiffRowDto {
                    key: "type".into(),
                    before: Some("playfile".into()),
                    after: "menu".into(),
                    kind: "changed".into(),
                },
                super::events::DiffRowDto {
                    key: "title".into(),
                    before: None,
                    after: "חדש".into(),
                    kind: "new".into(),
                },
            ],
            warnings: vec![],
            previous: None,
            snapshot_hash: Some("true:abc".into()),
        }
    }

    #[test]
    fn undo_values_come_from_the_diff_the_user_saw() {
        let undo = undo_params_of(&action_with_diff());
        assert_eq!(undo[0], ("type".to_string(), Some("playfile".to_string())));
        // a key that did not exist has no previous value
        assert_eq!(undo[1], ("title".to_string(), None));
    }

    #[tokio::test]
    async fn an_action_can_only_be_claimed_once() {
        let run = format!("r_test_{}", new_run_id());
        let client = Arc::new(YemotClient::new("t"));
        let ids = vec!["a_1".to_string(), "a_2".to_string()];

        assert!(claim_ids(&run, &client, &ids).await.is_empty());
        let second = claim_ids(&run, &client, &ids).await;
        assert_eq!(second.len(), 2, "a second approval must be refused");

        // a refused / undone action becomes available again
        release_ids(&run, &["a_1".to_string()]).await;
        let third = claim_ids(&run, &client, &ids).await;
        assert_eq!(third, HashSet::from(["a_2".to_string()]));

        write_state().lock().await.remove(&run);
    }

    #[tokio::test]
    async fn undo_records_are_kept_per_action() {
        let run = format!("r_test_{}", new_run_id());
        let client = Arc::new(YemotClient::new("t"));
        claim_ids(&run, &client, &["a_1".to_string()]).await;
        record_undo(
            &run,
            vec![UndoRecord {
                action_id: "a_1".into(),
                kind: "set_extension_params".into(),
                path: "/3".into(),
                canon_path: "ivr2:/3".into(),
                params: vec![("type".into(), Some("playfile".into()))],
                contents: None,
                post_hash: Some("true:beef".into()),
            }],
        )
        .await;
        let map = write_state().lock().await;
        let state = map.get(&run).unwrap();
        assert_eq!(state.undo["a_1"].params[0].1.as_deref(), Some("playfile"));
        // the canonical path stays internal, like everywhere else
        let v = serde_json::to_value(&state.undo["a_1"]).unwrap();
        assert!(v.get("canon_path").is_none());
        assert!(v.get("post_hash").is_none());
        drop(map);
        write_state().lock().await.remove(&run);
    }

    /// Two proposals on the same extension: approving them one at a time must
    /// work. The second one carried the pre-write snapshot and would otherwise
    /// be refused as stale by the write the first one just made.
    #[test]
    fn a_written_path_rebaselines_its_still_pending_proposals() {
        let mut first = action("a_1");
        first.snapshot_hash = Some("true:old".into());
        let mut second = action("a_2");
        second.snapshot_hash = Some("true:old".into());
        // same run, different extension — untouched
        let mut other = action("a_3");
        other.canon_path = "ivr2:/9".into();
        other.snapshot_hash = Some("true:old".into());
        // an upload on a path that happens to share the string is a different
        // kind of write and keeps its own baseline
        let mut upload = action("a_4");
        upload.kind = "upload_text_file".into();
        upload.snapshot_hash = Some("true:old".into());

        let mut pending = vec![first, second, other, upload];
        let done = HashSet::from(["a_1".to_string()]);
        let rehashed = vec![(
            "set_extension_params".to_string(),
            "ivr2:/3".to_string(),
            "true:new".to_string(),
        )];
        apply_rehashes(&mut pending, &done, &rehashed);

        // the applied action keeps what it had — its undo record is the record
        assert_eq!(pending[0].snapshot_hash.as_deref(), Some("true:old"));
        // the still-pending sibling now matches the server
        assert_eq!(pending[1].snapshot_hash.as_deref(), Some("true:new"));
        assert_eq!(pending[2].snapshot_hash.as_deref(), Some("true:old"));
        assert_eq!(pending[3].snapshot_hash.as_deref(), Some("true:old"));
    }

    /// Undo state holds a live Yemot client per run, so it is bounded by count
    /// as well as by age — the global map is never allowed to just grow.
    #[test]
    fn write_state_is_bounded_by_count_and_age() {
        fn empty_state(at: Instant) -> RunWriteState {
            RunWriteState {
                applied: HashSet::new(),
                undo: HashMap::new(),
                undone: HashSet::new(),
                log: Vec::new(),
                client: Arc::new(YemotClient::new("t")),
                at,
            }
        }
        let mut map: HashMap<String, RunWriteState> = HashMap::new();
        for i in 0..(MAX_WRITE_STATES + 10) {
            map.insert(format!("r_{}", i), empty_state(Instant::now()));
        }
        prune(&mut map);
        assert_eq!(map.len(), MAX_WRITE_STATES);

        // an entry past the retention window goes regardless of the count
        map.insert(
            "r_old".to_string(),
            empty_state(Instant::now() - UNDO_RETENTION - Duration::from_secs(1)),
        );
        prune(&mut map);
        assert!(!map.contains_key("r_old"));
    }

    #[test]
    fn run_id_is_prefixed() {
        assert!(new_run_id().starts_with("r_"));
        assert_ne!(new_run_id(), "r_0");
    }

    // -- retry budget ------------------------------------------------------

    #[test]
    fn a_backoff_that_outlasts_the_run_budget_is_not_slept() {
        assert!(budget_allows_retry(Duration::from_secs(10), 8000));
        // 599s spent + an 8s backoff would wake up past the deadline.
        assert!(!budget_allows_retry(RUN_BUDGET - Duration::from_secs(1), 8000));
        assert!(budget_allows_retry(RUN_BUDGET, 0));
        assert!(!budget_allows_retry(RUN_BUDGET + Duration::from_secs(1), 0));
    }

    // -- run lifecycle -----------------------------------------------------

    fn handle(finished: bool) -> (RunHandle, Arc<Mutex<Vec<ProposedAction>>>, Arc<AtomicBool>) {
        let proposed = Arc::new(Mutex::new(Vec::new()));
        let flag = Arc::new(AtomicBool::new(finished));
        (
            RunHandle {
                cancel: CancellationToken::new(),
                proposed: proposed.clone(),
                client: Arc::new(YemotClient::new("t")),
                finished: flag.clone(),
                transcript: Arc::new(Mutex::new(Vec::new())),
                audio: Arc::new(Mutex::new(HashMap::new())),
                attachments: Arc::new(Vec::new()),
                chain: Arc::new(Mutex::new(ChainState::default())),
            },
            proposed,
            flag,
        )
    }

    fn action(id: &str) -> ProposedAction {
        ProposedAction {
            id: id.to_string(),
            tool_use_id: "toolu_1".to_string(),
            kind: "set_extension_params".to_string(),
            path: "/3".to_string(),
            canon_path: "ivr2:/3".to_string(),
            params: vec![ActionParam { key: "type".into(), value: "menu".into() }],
            contents: None,
            reason: "בדיקה".to_string(),
            risk: "low".to_string(),
            exists: true,
            diff: Vec::new(),
            warnings: Vec::new(),
            previous: None,
            snapshot_hash: None,
        }
    }

    // -- continuation ------------------------------------------------------

    #[test]
    fn the_status_block_reports_applied_undone_and_unapproved() {
        let mut a1 = action("a_1");
        let mut a2 = action("a_2");
        a2.path = "/4".into();
        let mut a3 = action("a_3");
        a3.path = "/5".into();
        a1.path = "/3".into();

        let applied = HashSet::from(["a_1".to_string()]);
        let undone = HashSet::from(["a_2".to_string()]);
        let block = status_block(&[a1, a2, a3], &applied, &undone);
        assert_eq!(
            block,
            "[מצב ההצעות הקודמות]\n- a_1 /3: בוצע\n- a_2 /4: בוטל\n- a_3 /5: לא אושר\n"
        );

        // an undone action is released from `applied`, but stays "בוטל"
        let block = status_block(
            &[action("a_1")],
            &HashSet::new(),
            &HashSet::from(["a_1".to_string()]),
        );
        assert!(block.contains("בוטל"));

        assert!(status_block(&[], &HashSet::new(), &HashSet::new())
            .contains("לא הוצעו שינויים"));
    }

    #[test]
    fn a_continuation_carries_the_status_then_the_instruction() {
        let status = status_block(&[action("a_1")], &HashSet::new(), &HashSet::new());
        let text = continuation_text(&status, "  שנה גם את הכותרת  ", &[], &[]);
        assert!(text.starts_with("[מצב ההצעות הקודמות]\n"));
        assert!(text.ends_with("שנה גם את הכותרת"));
        assert!(!text.contains("[קבצים מצורפים]"));

        let with_files =
            continuation_text(&status, "העלה", &[attachment("f1", "a.mp3", 2048)], &[]);
        assert!(with_files.contains("[קבצים מצורפים]\n- f1: a.mp3 (2 KB, audio/mpeg)"));
    }

    #[test]
    fn a_continuation_reports_inherited_files_that_vanished() {
        let status = status_block(&[], &HashSet::new(), &HashSet::new());
        let text = continuation_text(
            &status,
            "המשך",
            &[],
            &[("a1".to_string(), "ברכה.mp3".to_string())],
        );
        assert!(text.contains("[קבצים מצורפים שאינם זמינים עוד]\n- a1: ברכה.mp3"));
        assert!(missing_attachments_block(&[]).is_none());
    }

    #[test]
    fn a_continuation_inherits_the_parents_attachments() {
        let (parent_file, parent_path) = temp_attachment("ברכה.mp3", 100);
        let (mut replacement, replacement_path) = temp_attachment("חדש.mp3", 200);
        replacement.id = parent_file.id.clone(); // same id, new file
        let (mut extra, extra_path) = temp_attachment("נוסף.mp3", 300);
        extra.id = "f2".to_string();

        let mut gone = parent_file.clone();
        gone.id = "f9".to_string();
        gone.local_path = format!("{}.nope", parent_file.local_path);

        // Nothing collides: the parent's file leads, the new one follows.
        let carried = carry_attachments(&[parent_file.clone()], &[extra.clone()]);
        let ids: Vec<&str> = carried.list.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["f1", "f2"], "the parent's ids must keep resolving");
        assert!(carried.missing.is_empty());
        assert_eq!(carried.fresh.len(), 1, "only new files are re-advertised");

        // Same id in the new payload: the file the user just picked wins.
        let carried = carry_attachments(&[parent_file.clone()], &[replacement.clone()]);
        assert_eq!(carried.list.len(), 1);
        assert_eq!(carried.list[0].name, "חדש.mp3");

        // A parent file that is gone from disk is dropped and reported.
        let carried = carry_attachments(&[parent_file.clone(), gone], &[]);
        let ids: Vec<&str> = carried.list.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["f1"]);
        assert_eq!(carried.missing, vec![("f9".to_string(), "ברכה.mp3".to_string())]);

        for p in [parent_path, replacement_path, extra_path] {
            let _ = std::fs::remove_dir_all(p.parent().unwrap());
        }
    }

    // -- attachments -------------------------------------------------------

    fn attachment(id: &str, name: &str, size: u64) -> Attachment {
        Attachment {
            id: id.to_string(),
            name: name.to_string(),
            local_path: String::new(),
            size,
            mime: "audio/mpeg".to_string(),
        }
    }

    fn temp_attachment(name: &str, bytes: usize) -> (Attachment, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("ai_yemot_test_{}", new_run_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, vec![0u8; bytes]).unwrap();
        (
            Attachment {
                id: "f1".to_string(),
                name: name.to_string(),
                local_path: path.to_string_lossy().to_string(),
                size: bytes as u64,
                mime: "audio/mpeg".to_string(),
            },
            path,
        )
    }

    #[test]
    fn attachments_are_validated_at_run_start() {
        let (ok, path) = temp_attachment("ברכה.mp3", 64);
        assert!(validate_attachments(&mut [ok.clone()]).is_ok());

        // a document is not an audio file
        let mut bad_ext = ok.clone();
        bad_ext.name = "רשימה.txt".to_string();
        assert!(validate_attachments(&mut [bad_ext]).is_err());

        // a plausible name with a non-audio mime is still refused
        let mut bad_mime = ok.clone();
        bad_mime.mime = "application/zip".to_string();
        assert!(validate_attachments(&mut [bad_mime]).is_err());

        // the file must exist and be a regular file
        let mut missing = ok.clone();
        missing.local_path = format!("{}.nope", ok.local_path);
        assert!(validate_attachments(&mut [missing]).is_err());
        let mut a_dir = ok.clone();
        a_dir.local_path = path.parent().unwrap().to_string_lossy().to_string();
        a_dir.name = "תיקיה.mp3".to_string();
        assert!(validate_attachments(&mut [a_dir]).is_err());

        // duplicate ids would make the id → path resolution ambiguous
        assert!(validate_attachments(&mut [ok.clone(), ok.clone()]).is_err());

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// The name may look like audio while the path that actually gets read does
    /// not — only the path is ever opened.
    #[test]
    fn the_local_path_is_extension_checked_too() {
        let (ok, path) = temp_attachment("ברכה.mp3", 64);
        let renamed = path.parent().unwrap().join("payload.exe");
        std::fs::rename(&path, &renamed).unwrap();
        let mut lying = ok.clone();
        lying.local_path = renamed.to_string_lossy().to_string();
        assert!(validate_attachments(&mut [lying]).is_err());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// The frontend's declared size is advisory — the block, the proposal
    /// params and the change log must all quote what is on disk.
    #[test]
    fn validation_fills_in_the_real_size_from_disk() {
        let (mut a, path) = temp_attachment("ברכה.mp3", 3_000);
        a.size = 0; // what the file picker reports
        let mut list = vec![a];
        validate_attachments(&mut list).unwrap();
        assert_eq!(list[0].size, 3_000);
        assert!(
            attachments_block(&list).unwrap().contains("(3 KB, audio/mpeg)"),
            "the model must see the real size, not 0 KB"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// The approval dialog may sit open for minutes; the file behind it can be
    /// swapped in that window.
    #[test]
    fn an_attachment_is_re_measured_before_the_bytes_are_read() {
        let (a, path) = temp_attachment("ברכה.mp3", 64);
        assert!(recheck_attachment(&a).is_ok());

        // grown past the cap since the proposal
        let f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        f.set_len(yemot::MAX_UPLOAD_BYTES + 1).unwrap();
        drop(f);
        let err = recheck_attachment(&a).unwrap_err();
        assert!(err.contains("גדול מ-25 MB"), "got {err}");

        // replaced by a directory
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir_all(&path).unwrap();
        assert!(recheck_attachment(&a).unwrap_err().contains("אינו קובץ"));

        // gone entirely
        std::fs::remove_dir_all(&path).unwrap();
        assert!(recheck_attachment(&a).is_err());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// The overwrite warning is decided when the action is proposed. If the
    /// slot fills up before the approval, the user approved a "new file" that
    /// would now destroy a recording nothing can restore.
    #[test]
    fn an_audio_slot_that_filled_up_since_the_proposal_is_refused() {
        let present = vec!["000.WAV".to_string(), "001.wav".to_string()];
        assert!(audio_slot_taken(false, &present, "000.wav"), "case-insensitive");
        assert!(!audio_slot_taken(false, &present, "002.wav"));
        // an overwrite the user already saw and approved stays approved
        assert!(!audio_slot_taken(true, &present, "000.wav"));
        assert_eq!(AUDIO_APPEARED_MSG, "הקובץ נוצר בשרת מאז ההצעה, הרץ שוב");
    }

    /// A cancelled run stays approvable — except for the one kind that cannot
    /// be undone.
    #[test]
    fn a_cancelled_run_refuses_audio_uploads_only() {
        let mut audio = action("a_2");
        audio.kind = "upload_audio_file".to_string();
        let picked = vec![action("a_1"), audio.clone()];

        let (keep, blocked) = split_cancelled_audio(false, picked.clone());
        assert_eq!(keep.len(), 2, "a live run applies everything it was given");
        assert!(blocked.is_empty());

        let (keep, blocked) = split_cancelled_audio(true, picked);
        assert_eq!(keep.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), vec!["a_1"]);
        assert_eq!(blocked.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), vec!["a_2"]);
        let r = refusal(&blocked[0], CANCELLED_AUDIO_MSG);
        assert!(!r.ok);
        assert_eq!(r.message, "הריצה בוטלה, הרץ שוב לפני העלאת שמע");
    }

    #[test]
    fn the_attachments_block_lists_only_ids_names_and_sizes() {
        assert!(attachments_block(&[]).is_none());
        let block = attachments_block(&[
            attachment("f1", "ברכה.mp3", 2_048),
            attachment("f2", "שיר.wav", 300),
        ])
        .unwrap();
        assert_eq!(
            block,
            "[קבצים מצורפים]\n- f1: ברכה.mp3 (2 KB, audio/mpeg)\n- f2: שיר.wav (1 KB, audio/mpeg)\n"
        );
        // the local path never reaches the model
        assert!(!block.contains("local_path"));
    }

    // -- change log --------------------------------------------------------

    #[test]
    fn change_labels_read_like_a_log_row() {
        assert_eq!(
            change_label("set_extension_params", "/1/2", "ivr2:/1/2", 3),
            "עדכון 3 הגדרות ב-/1/2"
        );
        assert_eq!(
            change_label("set_extension_params", "/3", "ivr2:/3", 1),
            "עדכון הגדרה אחת ב-/3"
        );
        assert_eq!(
            change_label("upload_audio_file", "ivr2:/1/000.wav", "ivr2:/1/000.wav", 2),
            "העלאת קובץ 000.wav ל-/1"
        );
        assert_eq!(
            change_label("upload_text_file", "ivr2:/000.tts", "ivr2:/000.tts", 0),
            "כתיבת קובץ 000.tts ל-/"
        );
    }

    #[tokio::test]
    async fn applied_changes_are_newest_first_and_carry_the_undone_flag() {
        let run = format!("r_test_{}", new_run_id());
        let client = Arc::new(YemotClient::new("t"));
        claim_ids(&run, &client, &["a_1".to_string(), "a_2".to_string()]).await;
        record_applied(
            &run,
            vec![
                AppliedRecord {
                    action_id: "a_1".into(),
                    kind: "set_extension_params".into(),
                    path: "/3".into(),
                    canon_path: "ivr2:/3".into(),
                    params: vec![ActionParam { key: "type".into(), value: "menu".into() }],
                    applied_at_ms: 1_000,
                    undo_available: true,
                },
                AppliedRecord {
                    action_id: "a_2".into(),
                    kind: "upload_audio_file".into(),
                    path: "ivr2:/1/000.wav".into(),
                    canon_path: "ivr2:/1/000.wav".into(),
                    params: vec![ActionParam { key: "file".into(), value: "000.wav".into() }],
                    applied_at_ms: 2_000,
                    undo_available: false,
                },
            ],
        )
        .await;
        // what `undo_action` does on success
        write_state().lock().await.get_mut(&run).unwrap().undone.insert("a_1".to_string());

        let all = list_applied_changes().await.unwrap();
        let mine: Vec<&AppliedChange> = all.iter().filter(|c| c.run_id == run).collect();
        assert_eq!(mine.len(), 2);
        // newest first
        assert_eq!(mine[0].action_id, "a_2");
        assert_eq!(mine[0].display, "/1/000.wav");
        assert_eq!(mine[0].label, "העלאת קובץ 000.wav ל-/1");
        // an audio upload is never undoable
        assert!(!mine[0].undo_available);
        assert!(!mine[0].undone);

        assert_eq!(mine[1].action_id, "a_1");
        assert_eq!(mine[1].label, "עדכון הגדרה אחת ב-/3");
        assert!(mine[1].undone);
        // an undone row cannot be undone a second time
        assert!(!mine[1].undo_available);

        // re-applying the same id replaces its row instead of duplicating it
        record_applied(
            &run,
            vec![AppliedRecord {
                action_id: "a_1".into(),
                kind: "set_extension_params".into(),
                path: "/3".into(),
                canon_path: "ivr2:/3".into(),
                params: vec![ActionParam { key: "type".into(), value: "menu".into() }],
                applied_at_ms: 3_000,
                undo_available: true,
            }],
        )
        .await;
        let all = list_applied_changes().await.unwrap();
        let mine: Vec<&AppliedChange> = all.iter().filter(|c| c.run_id == run).collect();
        assert_eq!(mine.len(), 2);
        assert_eq!(mine[0].action_id, "a_1");
        assert!(!mine[0].undone, "a fresh apply clears the undone mark");

        write_state().lock().await.remove(&run);
    }

    /// Everything approved in one click shares `applied_at_ms`, so the
    /// tie-break is the only thing ordering the batch — and `a_10` is newer
    /// than `a_2`, not older.
    #[tokio::test]
    async fn a_batch_applied_together_is_ordered_by_action_number() {
        let run = format!("r_test_{}", new_run_id());
        let client = Arc::new(YemotClient::new("t"));
        let row = |id: &str| AppliedRecord {
            action_id: id.to_string(),
            kind: "set_extension_params".into(),
            path: "/3".into(),
            canon_path: "ivr2:/3".into(),
            params: vec![ActionParam { key: "type".into(), value: "menu".into() }],
            applied_at_ms: 7_000,
            undo_available: true,
        };
        let ids = ["a_2".to_string(), "a_9".to_string(), "a_10".to_string()];
        claim_ids(&run, &client, &ids).await;
        record_applied(&run, ids.iter().map(|i| row(i)).collect()).await;

        let all = list_applied_changes().await.unwrap();
        let mine: Vec<&str> = all
            .iter()
            .filter(|c| c.run_id == run)
            .map(|c| c.action_id.as_str())
            .collect();
        assert_eq!(mine, vec!["a_10", "a_9", "a_2"]);
        assert_eq!(action_seq("a_10"), 10);
        assert_eq!(action_seq("weird"), 0, "an unparsable id must not panic");

        write_state().lock().await.remove(&run);
    }

    #[tokio::test]
    async fn a_finished_run_stays_approvable_until_the_next_run_starts() {
        let reg = AgentRegistry::default();
        let (h, proposed, flag) = handle(false);
        reg.claim("r_1", h).await.unwrap();

        // A second run is refused while the first is still going.
        let (h2, _, _) = handle(false);
        assert!(reg.claim("r_2", h2).await.is_err());

        // The run ends with actions waiting for approval.
        proposed.lock().await.push(action("a_1"));
        flag.store(true, Ordering::SeqCst);

        // The handle is still there — this is what `approve_actions` reads.
        {
            let runs = reg.runs.lock().await;
            let kept = runs.get("r_1").expect("finished handle must survive");
            assert!(kept.is_finished());
            assert_eq!(kept.proposed.lock().await.len(), 1);
        }

        // Starting a new run is allowed now, and evicts the finished handle.
        let (h3, _, _) = handle(false);
        reg.claim("r_3", h3).await.unwrap();
        let runs = reg.runs.lock().await;
        assert!(runs.get("r_1").is_none(), "the finished handle must be evicted");
        assert_eq!(runs.len(), 1);
        assert!(runs.contains_key("r_3"));
    }

    // --- task history ----------------------------------------------------

    /// The whole `continue_agent_run` history branch, minus the Tauri handle:
    /// a saved record must rebuild exactly the `initial` messages a live parent
    /// would have produced — its transcript, then one user message carrying the
    /// status of its proposals and the new instruction.
    #[test]
    fn a_continuation_from_history_replays_the_record() {
        use crate::agent::history::{self, TaskRecord, TaskUsage};

        let dir = std::env::temp_dir().join(format!("ai_yemot_cont_{}", new_run_id()));
        std::fs::create_dir_all(&dir).unwrap();

        let applied_action = action("a_1");
        let pending_action = action("a_2");
        let rec = TaskRecord {
            task_id: "r_first".into(),
            last_run_id: "r_second".into(),
            created_at_ms: 1,
            updated_at_ms: 2,
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
                    content: vec![ContentBlock::Text("הצעתי שינוי".into())],
                },
            ],
            attachments: Vec::new(),
            actions: vec![applied_action, pending_action],
            applied_action_ids: vec!["a_1".into()],
            usage: TaskUsage::default(),
        };
        history::save(&dir, &rec).unwrap();

        // …exactly what the command does: resolve the parent by its run id
        let loaded = history::load_for_parent(&dir, "r_second").expect("parent record");
        let applied: HashSet<String> = loaded.applied_action_ids.iter().cloned().collect();
        let status = status_block(&loaded.actions, &applied, &HashSet::new());
        let mut initial = loaded.transcript.clone();
        initial.push(Message::user_text(continuation_text(
            &status,
            "עכשיו הוסף הודעת פתיחה",
            &[],
            &[],
        )));

        assert_eq!(initial.len(), 3, "the parent transcript plus one message");
        let last = match &initial[2].content[0] {
            ContentBlock::Text(t) => t.clone(),
            other => panic!("wrong block: {:?}", other),
        };
        assert!(last.contains("[מצב ההצעות הקודמות]"));
        assert!(last.contains("a_1 /3: בוצע"));
        assert!(last.contains("a_2 /3: לא אושר"));
        assert!(last.contains("עכשיו הוסף הודעת פתיחה"));
        // the chain continues under the parent's task id, one instruction longer
        assert_eq!(loaded.task_id, "r_first");
        let mut chain = ChainState {
            task_id: loaded.task_id.clone(),
            created_at_ms: loaded.created_at_ms,
            instructions: loaded.instructions.clone(),
            usage: loaded.usage.clone(),
        };
        chain.instructions.push("עכשיו הוסף הודעת פתיחה".into());
        assert_eq!(chain.instructions.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_is_saved_unless_the_payload_turns_it_off() {
        let base = serde_json::json!({
            "provider": "claude", "model": "regular", "prompt": "x",
            "api_key": "k", "yemot_token": "t"
        });
        let on: AgentRunPayload = serde_json::from_value(base.clone()).unwrap();
        assert!(on.save_history, "history is on unless the user turns it off");

        let mut off = base;
        off["save_history"] = serde_json::json!(false);
        let off: AgentRunPayload = serde_json::from_value(off).unwrap();
        assert!(!off.save_history);

        // and the gate the save point uses says so too
        let transcript = vec![Message::user_text("x")];
        assert!(crate::agent::history::should_save(on.save_history, &transcript));
        assert!(!crate::agent::history::should_save(off.save_history, &transcript));
    }

    #[test]
    fn a_run_started_carries_its_task_id() {
        let v = serde_json::to_value(AgentRunStarted {
            run_id: "r_2".into(),
            task_id: "r_1".into(),
        })
        .unwrap();
        assert_eq!(v["run_id"], serde_json::json!("r_2"));
        assert_eq!(v["task_id"], serde_json::json!("r_1"));
    }
}
