//! The agent loop and its three Tauri commands.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicUsize;
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

/// Above this the oldest tool results are collapsed; above the hard limit the
/// run stops rather than paying for a prompt that no longer fits its purpose.
pub const CONTEXT_SOFT_LIMIT: usize = 60_000;
pub const CONTEXT_HARD_LIMIT: usize = 150_000;
/// Collapsing rewrites the cached prefix, so it may not happen every turn.
pub const COLLAPSE_EVERY_N_TURNS: u32 = 3;
/// Tool results of the last two turns are always kept verbatim.
const KEEP_LAST_TURNS: usize = 2;

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

fn placeholder(name: &str, content: &str) -> String {
    let head: String = content
        .chars()
        .take(60)
        .map(|c| if c == '\n' { ' ' } else { c })
        .collect();
    format!("(תוצאה קוצרה: {} {})", name, head.trim())
}

/// Collapse the *content* of tool results older than the last `KEEP_LAST_TURNS`
/// exchanges. The blocks themselves stay: dropping one half of a
/// tool_use/tool_result pair is a 400 on every provider.
/// Returns how many results were collapsed.
pub fn collapse_old_tool_results(messages: &mut [Message], keep_last_turns: usize) -> usize {
    let protected = keep_last_turns * 2;
    if messages.len() <= protected + 1 {
        return 0;
    }
    let cutoff = messages.len() - protected;
    let mut collapsed = 0;
    for m in messages.iter_mut().take(cutoff) {
        for b in m.content.iter_mut() {
            if let ContentBlock::ToolResult { name, content, .. } = b {
                if content.starts_with("(תוצאה קוצרה:") {
                    continue;
                }
                *content = placeholder(name, content);
                collapsed += 1;
            }
        }
    }
    collapsed
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct RunHandle {
    pub cancel: CancellationToken,
    pub proposed: Arc<Mutex<Vec<ProposedAction>>>,
    pub client: Arc<YemotClient>,
}

#[derive(Default)]
pub struct AgentRegistry {
    pub runs: Mutex<HashMap<String, RunHandle>>,
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
    payload: AgentRunPayload,
) -> Result<AgentRunStarted, String> {
    if payload.prompt.trim().is_empty() {
        return Err("לא הוזנה בקשה".to_string());
    }
    if payload.yemot_token.trim().is_empty() {
        return Err("נדרשת התחברות למערכת ימות המשיח".to_string());
    }
    let provider = providers::build(
        &payload.provider,
        &payload.model,
        &payload.api_key,
        &payload.base_url,
    )?;
    let model = providers::resolve_model(&payload.provider, &payload.model);

    let mut runs = registry.runs.lock().await;
    if !runs.is_empty() {
        return Err("ריצה אחרת פעילה כרגע. בטל אותה לפני התחלת ריצה חדשה.".to_string());
    }

    let run_id = new_run_id();
    let cancel = CancellationToken::new();
    let proposed = Arc::new(Mutex::new(Vec::new()));
    let client = Arc::new(YemotClient::new(payload.yemot_token.clone()));
    runs.insert(
        run_id.clone(),
        RunHandle {
            cancel: cancel.clone(),
            proposed: proposed.clone(),
            client: client.clone(),
        },
    );
    drop(runs);

    let ctx = RunContext {
        app: app.clone(),
        run_id: run_id.clone(),
        provider_name: payload.provider.clone(),
        model,
        prompt: payload.prompt.clone(),
        auto_apply: payload.auto_apply,
        include_tree: payload.include_tree,
        client,
        proposed,
        cancel,
    };

    tauri::async_runtime::spawn(async move {
        let id = ctx.run_id.clone();
        let handle = ctx.app.clone();
        run_loop(ctx, provider).await;
        // A finished run must free the slot even if it ended badly.
        if let Some(reg) = tauri::Manager::try_state::<AgentRegistry>(&handle) {
            reg.runs.lock().await.remove(&id);
        }
    });

    Ok(AgentRunStarted { run_id })
}

#[tauri::command]
pub async fn cancel_agent_run(
    registry: State<'_, AgentRegistry>,
    run_id: String,
) -> Result<(), String> {
    let runs = registry.runs.lock().await;
    match runs.get(&run_id) {
        Some(h) => {
            h.cancel.cancel();
            Ok(())
        }
        None => Err("הריצה כבר הסתיימה".to_string()),
    }
}

#[tauri::command]
pub async fn approve_actions(
    app: AppHandle,
    registry: State<'_, AgentRegistry>,
    run_id: String,
    action_ids: Vec<String>,
) -> Result<Vec<ActionApplyResult>, String> {
    let (client, actions) = {
        let runs = registry.runs.lock().await;
        let handle = runs
            .get(&run_id)
            .ok_or_else(|| "הריצה כבר אינה זמינה — הרץ שוב".to_string())?;
        let all = handle.proposed.lock().await.clone();
        let wanted: HashSet<&String> = action_ids.iter().collect();
        let picked: Vec<ProposedAction> =
            all.into_iter().filter(|a| wanted.contains(&a.id)).collect();
        (handle.client.clone(), picked)
    };
    if actions.is_empty() {
        return Err("לא נבחרו פעולות לביצוע".to_string());
    }
    let results = apply_actions(&client, &actions).await;
    for r in &results {
        events::action_applied(&app, &run_id, r);
    }
    Ok(results)
}

/// Group by extension so each path costs exactly one `UpdateExtension`.
async fn apply_actions(client: &YemotClient, actions: &[ProposedAction]) -> Vec<ActionApplyResult> {
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

    let mut per_path: HashMap<String, Result<Vec<ParamOutcome>, String>> = HashMap::new();
    for path in &order {
        let params = grouped.get(path).cloned().unwrap_or_default();
        let res = client
            .update_extension(path, &params)
            .await
            .map_err(|e| yemot::render_error(&e));
        per_path.insert(path.clone(), res);
    }

    let mut out = Vec::new();
    for a in actions {
        if a.kind == "upload_text_file" {
            let contents = a.contents.clone().unwrap_or_default();
            let r = client.upload_text_file(&a.canon_path, &contents).await;
            out.push(match r {
                Ok(_) => ActionApplyResult {
                    action_id: a.id.clone(),
                    ok: true,
                    message: format!("הקובץ {} נכתב", a.path),
                    params: Vec::new(),
                },
                Err(e) => ActionApplyResult {
                    action_id: a.id.clone(),
                    ok: false,
                    message: yemot::render_error(&e),
                    params: Vec::new(),
                },
            });
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
                out.push(ActionApplyResult {
                    action_id: a.id.clone(),
                    ok,
                    message: yemot::render_outcomes(&a.canon_path, &mine),
                    params: mine,
                });
            }
            Some(Err(msg)) => out.push(ActionApplyResult {
                action_id: a.id.clone(),
                ok: false,
                message: msg.clone(),
                params: Vec::new(),
            }),
            None => out.push(ActionApplyResult {
                action_id: a.id.clone(),
                ok: false,
                message: "הפעולה לא נמצאה".to_string(),
                params: Vec::new(),
            }),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The loop
// ---------------------------------------------------------------------------

struct RunContext {
    app: AppHandle,
    run_id: String,
    provider_name: String,
    model: String,
    prompt: String,
    auto_apply: bool,
    include_tree: bool,
    client: Arc<YemotClient>,
    proposed: Arc<Mutex<Vec<ProposedAction>>>,
    cancel: CancellationToken,
}

async fn run_loop(ctx: RunContext, provider: Box<dyn Provider>) {
    let started_at = Instant::now();
    events::started(&ctx.app, &ctx.run_id, &ctx.provider_name, &ctx.model);

    let tools = tools::tool_specs();
    let system = prompt::system_blocks(&ctx.provider_name);

    let mut first = ctx.prompt.clone();
    if ctx.include_tree {
        if let Some(block) = root_tree_block(&ctx.client).await {
            first.push_str("\n\n");
            first.push_str(&block);
        }
    }
    let mut messages: Vec<Message> = vec![Message::user_text(first)];

    let tool_ctx = ToolCtx {
        app: ctx.app.clone(),
        run_id: ctx.run_id.clone(),
        client: ctx.client.clone(),
        auto_apply: ctx.auto_apply,
        reads: Arc::new(Mutex::new(HashSet::new())),
        proposed: ctx.proposed.clone(),
        action_seq: Arc::new(AtomicUsize::new(0)),
        session_error: Arc::new(Mutex::new(None)),
    };

    let mut usage = Usage::default();
    let mut turns_done: u32 = 0;
    let mut tool_calls: u32 = 0;
    let mut mutating_calls: u32 = 0;
    let mut final_text = String::new();
    let mut stop = "max_turns";
    let mut ok = true;
    let mut guard = LoopGuard::default();
    let mut last_collapse_turn: u32 = 0;

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

        // Context hygiene, before the prompt is assembled.
        let size = estimate_context(&system, &messages);
        if size > CONTEXT_HARD_LIMIT {
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
        if size > CONTEXT_SOFT_LIMIT
            && (last_collapse_turn == 0 || turn - last_collapse_turn >= COLLAPSE_EVERY_N_TURNS)
            && collapse_old_tool_results(&mut messages, KEEP_LAST_TURNS) > 0
        {
            last_collapse_turn = turn;
        }

        events::turn_start(&ctx.app, &ctx.run_id, turn, MAX_TURNS);

        let req = ProviderRequest {
            model: ctx.model.clone(),
            system: system.clone(),
            messages: messages.clone(),
            tools: tools.clone(),
            turn,
        };

        let response = match call_with_retry(&ctx, provider.as_ref(), &req).await {
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

    let run_usage = RunUsage {
        input_tokens: usage.input,
        output_tokens: usage.output,
        cache_read_tokens: usage.cache_read,
        cache_write_tokens: usage.cache_write,
        turns: turns_done,
        tool_calls,
        elapsed_ms: started_at.elapsed().as_millis() as u64,
        cache_hit_pct: usage.cache_hit_pct(),
        cost_usd: pricing::cost_usd(&ctx.model, &usage),
    };
    events::finished(&ctx.app, &ctx.run_id, ok, stop, &final_text, &run_usage);
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

async fn call_with_retry(
    ctx: &RunContext,
    provider: &dyn Provider,
    req: &ProviderRequest,
) -> Result<ProviderResponse, ProviderError> {
    let mut last: ProviderError = ProviderError::Transient("—".to_string());
    for attempt in 1..=MAX_ATTEMPTS {
        match provider.complete(req, &ctx.cancel).await {
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
    fn collapse_keeps_the_last_two_turns_and_both_halves_of_every_pair() {
        let mut messages = vec![Message::user_text("בקשה")];
        for i in 0..4 {
            messages.extend(tool_pair(
                &format!("t{}", i),
                "search_knowledge",
                &format!("תוצאה ארוכה מאוד מספר {} {}", i, "y".repeat(500)),
            ));
        }
        let before_len = messages.len();
        let n = collapse_old_tool_results(&mut messages, 2);
        assert_eq!(messages.len(), before_len, "no message may be dropped");
        assert_eq!(n, 2, "the last two exchanges keep their results verbatim");

        let bodies: Vec<String> = messages
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(|b| match b {
                ContentBlock::ToolResult { content, .. } => Some(content.clone()),
                _ => None,
            })
            .collect();
        assert!(bodies[0].starts_with("(תוצאה קוצרה: search_knowledge"));
        assert!(bodies[0].chars().count() < 110);
        assert!(bodies[1].starts_with("(תוצאה קוצרה: search_knowledge"));
        assert!(bodies[2].contains("yyyy"), "the last two results stay verbatim");
        assert!(bodies[3].contains("yyyy"));

        // every tool_use still has its tool_result
        let uses = messages
            .iter()
            .flat_map(|m| m.content.iter())
            .filter(|b| matches!(b, ContentBlock::ToolUse { .. }))
            .count();
        assert_eq!(uses, 4);

        // and collapsing twice is a no-op
        assert_eq!(collapse_old_tool_results(&mut messages, 2), 0);
    }

    #[test]
    fn collapse_does_nothing_on_a_short_conversation() {
        let mut messages = vec![Message::user_text("בקשה")];
        messages.extend(tool_pair("t0", "lookup_param", "type=menu"));
        assert_eq!(collapse_old_tool_results(&mut messages, 2), 0);
    }

    #[test]
    fn context_limits_are_ordered() {
        assert_eq!(CONTEXT_SOFT_LIMIT, 60_000);
        assert_eq!(CONTEXT_HARD_LIMIT, 150_000);
    }

    #[test]
    fn run_id_is_prefixed() {
        assert!(new_run_id().starts_with("r_"));
        assert_ne!(new_run_id(), "r_0");
    }
}
