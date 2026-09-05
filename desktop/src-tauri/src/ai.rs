//! Legacy single-shot AI command.
//!
//! `target_mode = "script"` still calls the Google Apps Script webhook exactly
//! as before. `target_mode = "direct"` no longer has its own HTTP code or its
//! own prompt: it goes through `agent::providers` with the real system prompt,
//! but with **no tools** — one question, one answer. The agentic path is
//! `agent::start_agent_run` (see `docs/agent-contract.md`).
//!
//! The Yemot token is not part of this payload at all: nothing on this path
//! ever needs it, and what is not carried cannot leak.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::agent::prompt;
use crate::agent::providers::{self, http_ai};
use crate::agent::types::{Message, ProviderRequest};

const SCRIPT_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AIRequestPayload {
    pub target_mode: String, // "script" or "direct"
    pub provider: String,    // "gemini", "openai", "claude", "groq", "custom"
    pub model: String,       // "regular", "pro", or an explicit model id
    pub prompt: String,      // User instruction
    pub api_key: String,     // Provider API key (personal / BYOK)
    pub is_preview: bool,    // Preview / FullAnswer
    pub logout: bool,        // Invalidate token after run (handled locally)
    pub script_url: String,  // Google Apps Script or Webhook URL
    #[serde(default)]
    pub base_url: String, // Optional custom API URL (direct mode)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AIResponsePayload {
    pub success: bool,
    pub message: String,
    pub raw_response: String,
    pub is_preview: bool,
}

#[tauri::command]
pub async fn send_ai_request(payload: AIRequestPayload) -> Result<AIResponsePayload, String> {
    if payload.target_mode == "script" {
        send_to_script(payload).await
    } else {
        send_to_direct_ai(payload).await
    }
}

async fn send_to_script(payload: AIRequestPayload) -> Result<AIResponsePayload, String> {
    if payload.script_url.trim().is_empty() {
        return Err("כתובת הסקריפט (Script URL) לא הוגדרה".to_string());
    }

    // SECURITY: the Yemot token is never sent to the script — only the AI
    // prompt (and optional personal API key) leave this machine. All Yemot
    // actions are executed locally by this app.
    let mut query_params: Vec<(&str, String)> = Vec::new();
    query_params.push(("text", payload.prompt.clone()));

    if payload.model.to_lowercase() == "pro" {
        query_params.push(("model", "pro".to_string()));
    }
    if !payload.api_key.trim().is_empty() {
        query_params.push(("key", payload.api_key.clone()));
    }
    // Logout is handled locally (logout_yemot command), never by the script.
    if payload.is_preview {
        query_params.push(("Fullanswer", "yes".to_string()));
    }

    let request = http_ai()
        .get(&payload.script_url)
        .query(&query_params)
        .header("Accept", "application/json, text/plain, */*")
        .send();

    let res = tokio::time::timeout(SCRIPT_TIMEOUT, request)
        .await
        .map_err(|_| "פסק זמן בהמתנה לתשובת הסקריפט".to_string())?
        .map_err(|e| format!("שגיאת תקשורת מול הסקריפט: {}", e.without_url()))?;

    let status = res.status();
    let text = res
        .text()
        .await
        .map_err(|e| format!("שגיאת קריאת תשובה: {}", e.without_url()))?;

    if status.is_success() {
        Ok(AIResponsePayload {
            success: true,
            message: "הבקשה עובדה בהצלחה ע\"י הסקריפט".to_string(),
            raw_response: text,
            is_preview: payload.is_preview,
        })
    } else {
        Err(format!("שגיאת שרת סקריפט ({}): {}", status, text))
    }
}

/// One turn, no tools, the same system prompt the agent uses. Kept so the old
/// "ask a question" UI keeps working; anything that must *change* the system
/// goes through `start_agent_run`.
async fn send_to_direct_ai(payload: AIRequestPayload) -> Result<AIResponsePayload, String> {
    let provider = providers::build(
        &payload.provider,
        &payload.model,
        &payload.api_key,
        &payload.base_url,
    )?;
    let model = providers::resolve_model(&payload.provider, &payload.model);

    let req = ProviderRequest {
        model: model.clone(),
        system: prompt::system_blocks(&payload.provider),
        messages: vec![Message::user_text(payload.prompt.clone())],
        tools: Vec::new(),
        turn: 1,
    };

    let response = provider
        .complete(&req, &CancellationToken::new())
        .await
        .map_err(|e| e.message())?;

    let text = response.text();
    Ok(AIResponsePayload {
        success: true,
        message: format!("תשובה התקבלה מ-{}", model),
        raw_response: if text.trim().is_empty() {
            "לא התקבלה תשובה טקסטואלית".to_string()
        } else {
            text
        },
        is_preview: payload.is_preview,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_no_longer_carries_the_yemot_token() {
        let json = serde_json::to_value(AIRequestPayload {
            target_mode: "direct".into(),
            provider: "claude".into(),
            model: "regular".into(),
            prompt: "p".into(),
            api_key: "k".into(),
            is_preview: false,
            logout: false,
            script_url: String::new(),
            base_url: String::new(),
        })
        .unwrap();
        assert!(json.get("token").is_none());
        assert!(json.get("yemot_token").is_none());
    }
}
