//! The wire shapes of `docs/agent-contract.md`.
//!
//! Field names here are the contract. The frontend is already written against
//! them verbatim — renaming one is a breaking change, not a refactor.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::yemot::ParamOutcome;
use crate::yemot_ini::{DiffKind, DiffRow};

// ---------------------------------------------------------------------------
// Proposed actions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffRowDto {
    pub key: String,
    pub before: Option<String>,
    pub after: String,
    /// `changed` | `new` | `unchanged`
    pub kind: String,
}

impl From<&DiffRow> for DiffRowDto {
    fn from(r: &DiffRow) -> Self {
        DiffRowDto {
            key: r.key.clone(),
            before: r.before.clone(),
            after: r.after.clone(),
            kind: match r.kind {
                DiffKind::New => "new",
                DiffKind::Changed => "changed",
                DiffKind::Unchanged => "unchanged",
            }
            .to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionParam {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposedAction {
    pub id: String,
    pub tool_use_id: String,
    /// `set_extension_params` | `upload_text_file`
    pub kind: String,
    /// Display form (`/3`); `canon_path` keeps the `ivr2:` form for execution.
    pub path: String,
    #[serde(skip)]
    pub canon_path: String,
    pub params: Vec<ActionParam>,
    pub contents: Option<String>,
    pub reason: String,
    /// `low` | `overwrite` | `destructive`
    pub risk: String,
    pub exists: bool,
    pub diff: Vec<DiffRowDto>,
    pub warnings: Vec<String>,
    /// Previous contents of the file an `upload_text_file` would overwrite
    /// (`None` when the file does not exist) — the UI diff and the undo.
    #[serde(default)]
    pub previous: Option<String>,
    /// Fingerprint of the `ext.ini` as read when the action was proposed; the
    /// approval refuses to write when the file changed on the server since.
    #[serde(default)]
    pub snapshot_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionApplyResult {
    pub action_id: String,
    pub ok: bool,
    pub message: String,
    pub params: Vec<ParamOutcome>,
    /// What `undo_action` would restore (`runner::UndoRecord`), or `None` when
    /// the action cannot be undone.
    #[serde(default)]
    pub undo: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub turns: u32,
    pub tool_calls: u32,
    pub elapsed_ms: u64,
    pub cache_hit_pct: f64,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRunStarted {
    pub run_id: String,
}

// ---------------------------------------------------------------------------
// Emitters — one per contract row, so a typo cannot spread
// ---------------------------------------------------------------------------

fn emit(app: &AppHandle, event: &str, payload: serde_json::Value) {
    let _ = app.emit(event, payload);
}

pub fn started(app: &AppHandle, run_id: &str, provider: &str, model: &str) {
    emit(
        app,
        "agent:started",
        serde_json::json!({ "run_id": run_id, "provider": provider, "model": model }),
    );
}

pub fn turn_start(app: &AppHandle, run_id: &str, turn: u32, max_turns: u32) {
    emit(
        app,
        "agent:turn_start",
        serde_json::json!({ "run_id": run_id, "turn": turn, "max_turns": max_turns }),
    );
}

pub fn assistant_text(app: &AppHandle, run_id: &str, turn: u32, text: &str) {
    emit(
        app,
        "agent:assistant_text",
        serde_json::json!({ "run_id": run_id, "turn": turn, "text": text }),
    );
}

pub fn tool_started(app: &AppHandle, run_id: &str, tool_use_id: &str, name: &str, label: &str) {
    emit(
        app,
        "agent:tool_started",
        serde_json::json!({
            "run_id": run_id, "tool_use_id": tool_use_id, "name": name, "label": label
        }),
    );
}

pub fn tool_finished(
    app: &AppHandle,
    run_id: &str,
    tool_use_id: &str,
    ok: bool,
    summary: &str,
    ms: u64,
) {
    emit(
        app,
        "agent:tool_finished",
        serde_json::json!({
            "run_id": run_id, "tool_use_id": tool_use_id, "ok": ok,
            "summary": summary, "ms": ms
        }),
    );
}

pub fn action_proposed(app: &AppHandle, run_id: &str, action: &ProposedAction) {
    emit(
        app,
        "agent:action_proposed",
        serde_json::json!({ "run_id": run_id, "action": action }),
    );
}

pub fn actions_proposed(app: &AppHandle, run_id: &str, actions: &[ProposedAction]) {
    emit(
        app,
        "agent:actions_proposed",
        serde_json::json!({ "run_id": run_id, "actions": actions }),
    );
}

pub fn action_applied(app: &AppHandle, run_id: &str, r: &ActionApplyResult) {
    emit(
        app,
        "agent:action_applied",
        serde_json::json!({
            "run_id": run_id, "action_id": r.action_id, "ok": r.ok,
            "message": r.message, "params": r.params
        }),
    );
}

pub fn retry(app: &AppHandle, run_id: &str, attempt: u32, max: u32, reason: &str, wait_ms: u64) {
    emit(
        app,
        "agent:retry",
        serde_json::json!({
            "run_id": run_id, "attempt": attempt, "max": max,
            "reason": reason, "wait_ms": wait_ms
        }),
    );
}

pub fn finished(app: &AppHandle, run_id: &str, ok: bool, stop: &str, final_text: &str, usage: &RunUsage) {
    emit(
        app,
        "agent:finished",
        serde_json::json!({
            "run_id": run_id, "ok": ok, "stop": stop,
            "final_text": final_text, "usage": usage
        }),
    );
}

pub fn error(app: &AppHandle, run_id: &str, code: &str, message: &str) {
    emit(
        app,
        "agent:error",
        serde_json::json!({ "run_id": run_id, "code": code, "message": message }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proposed_action_serializes_with_contract_field_names() {
        let a = ProposedAction {
            id: "a_1".into(),
            tool_use_id: "toolu_1".into(),
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
        };
        let v = serde_json::to_value(&a).unwrap();
        for f in [
            "id", "tool_use_id", "kind", "path", "params", "contents", "reason", "risk",
            "exists", "diff", "warnings",
        ] {
            assert!(v.get(f).is_some(), "missing contract field {}", f);
        }
        // the canonical path is internal and must not reach the frontend
        assert!(v.get("canon_path").is_none());
        assert_eq!(v["diff"][0]["kind"], serde_json::json!("changed"));
    }

    #[test]
    fn run_usage_field_names() {
        let u = RunUsage {
            input_tokens: 1,
            output_tokens: 2,
            cache_read_tokens: 3,
            cache_write_tokens: 4,
            turns: 5,
            tool_calls: 6,
            elapsed_ms: 7,
            cache_hit_pct: 8.0,
            cost_usd: None,
        };
        let v = serde_json::to_value(&u).unwrap();
        for f in [
            "input_tokens", "output_tokens", "cache_read_tokens", "cache_write_tokens",
            "turns", "tool_calls", "elapsed_ms", "cache_hit_pct", "cost_usd",
        ] {
            assert!(v.get(f).is_some(), "missing usage field {}", f);
        }
        assert!(v["cost_usd"].is_null());
    }
}
