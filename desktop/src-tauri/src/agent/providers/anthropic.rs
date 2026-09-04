//! Anthropic Messages API.
//!
//! Cache discipline: `tools` + both `system` blocks are byte-identical on every
//! turn, the second (and last) system block carries the static breakpoint, and
//! exactly one moving breakpoint sits on the last content block of the last
//! message. Nothing else in the prefix ever changes.

use async_trait::async_trait;
use serde_json::{json, Map, Value};
use tokio_util::sync::CancellationToken;

use super::super::types::{
    ContentBlock, Message, ProviderError, ProviderRequest, ProviderResponse, StopReason, ToolSpec,
    Usage,
};
use super::{call_with_deadline, classify_reqwest, classify_status, http_ai, retry_after_of};

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
const MAX_TOKENS: u64 = 8000;

pub struct Anthropic {
    api_key: String,
    base_url: String,
}

impl Anthropic {
    /// The model is not stored: `ProviderRequest::model` is authoritative.
    pub fn new(api_key: String, base_url: String) -> Self {
        Anthropic { api_key, base_url }
    }

    fn endpoint(&self) -> String {
        if self.base_url.is_empty() {
            API_URL.to_string()
        } else if self.base_url.ends_with("/messages") {
            self.base_url.clone()
        } else {
            format!("{}/v1/messages", self.base_url)
        }
    }
}

/// Opus-class models opt into server-side fallback on a policy refusal.
pub fn wants_fallback(model: &str) -> bool {
    model.to_ascii_lowercase().contains("opus")
}

// ---------------------------------------------------------------------------
// Request mapping
// ---------------------------------------------------------------------------

pub fn tools_json(tools: &[ToolSpec]) -> Value {
    Value::Array(
        tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "strict": true,
                    "input_schema": t.input_schema,
                })
            })
            .collect(),
    )
}

pub fn system_json(system: &[String]) -> Value {
    let last = system.len().saturating_sub(1);
    Value::Array(
        system
            .iter()
            .enumerate()
            .map(|(i, text)| {
                let mut o = Map::new();
                o.insert("type".to_string(), json!("text"));
                o.insert("text".to_string(), json!(text));
                if i == last {
                    o.insert("cache_control".to_string(), json!({ "type": "ephemeral" }));
                }
                Value::Object(o)
            })
            .collect(),
    )
}

fn block_json(b: &ContentBlock) -> Value {
    match b {
        ContentBlock::Text(t) => json!({ "type": "text", "text": t }),
        ContentBlock::ToolUse { id, name, input } => {
            json!({ "type": "tool_use", "id": id, "name": name, "input": input })
        }
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
            ..
        } => json!({
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "content": content,
            "is_error": is_error,
        }),
        // Thinking blocks must go back exactly as they arrived.
        ContentBlock::Thinking(raw) => raw.clone(),
    }
}

pub fn messages_json(messages: &[Message]) -> Value {
    let mut out: Vec<Value> = messages
        .iter()
        .map(|m| {
            json!({
                "role": m.role.as_str(),
                "content": Value::Array(m.content.iter().map(block_json).collect()),
            })
        })
        .collect();

    // The single moving cache breakpoint: last content block of the last message.
    if let Some(last_msg) = out.last_mut() {
        if let Some(blocks) = last_msg.get_mut("content").and_then(|c| c.as_array_mut()) {
            if let Some(last) = blocks.last_mut() {
                if let Some(o) = last.as_object_mut() {
                    o.insert("cache_control".to_string(), json!({ "type": "ephemeral" }));
                }
            }
        }
    }
    Value::Array(out)
}

pub fn build_body(req: &ProviderRequest) -> Value {
    let mut body = Map::new();
    body.insert("model".to_string(), json!(req.model));
    body.insert("max_tokens".to_string(), json!(MAX_TOKENS));
    body.insert("thinking".to_string(), json!({ "type": "adaptive" }));
    body.insert("output_config".to_string(), json!({ "effort": "medium" }));
    body.insert("tools".to_string(), tools_json(&req.tools));
    body.insert("system".to_string(), system_json(&req.system));
    body.insert("messages".to_string(), messages_json(&req.messages));
    if wants_fallback(&req.model) {
        body.insert("fallbacks".to_string(), json!("default"));
    }
    Value::Object(body)
}

// ---------------------------------------------------------------------------
// Response mapping
// ---------------------------------------------------------------------------

pub fn parse_stop(raw: Option<&str>) -> StopReason {
    match raw {
        Some("end_turn") | Some("stop_sequence") => StopReason::EndTurn,
        Some("tool_use") | Some("pause_turn") => StopReason::ToolUse,
        Some("max_tokens") => StopReason::MaxTokens,
        Some("refusal") => StopReason::Refusal,
        Some(other) => StopReason::Other(other.to_string()),
        None => StopReason::EndTurn,
    }
}

pub fn parse_response(json: &Value) -> ProviderResponse {
    let mut content = Vec::new();
    if let Some(items) = json.get("content").and_then(|c| c.as_array()) {
        for item in items {
            match item.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                "text" => content.push(ContentBlock::Text(
                    item.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string(),
                )),
                "tool_use" => content.push(ContentBlock::ToolUse {
                    id: item.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    name: item.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    input: item.get("input").cloned().unwrap_or_else(|| json!({})),
                }),
                "thinking" | "redacted_thinking" => {
                    content.push(ContentBlock::Thinking(item.clone()))
                }
                _ => {}
            }
        }
    }

    let u = json.get("usage");
    let n = |k: &str| -> u64 {
        u.and_then(|u| u.get(k)).and_then(|v| v.as_u64()).unwrap_or(0)
    };

    ProviderResponse {
        // Never branch on the stop reason before the tool calls are read out:
        // the runner decides from `tool_uses()`, not from `stop`.
        stop: parse_stop(json.get("stop_reason").and_then(|v| v.as_str())),
        content,
        usage: Usage {
            input: n("input_tokens"),
            output: n("output_tokens"),
            cache_read: n("cache_read_input_tokens"),
            cache_write: n("cache_creation_input_tokens"),
        },
    }
}

// ---------------------------------------------------------------------------

#[async_trait]
impl Provider for Anthropic {
    fn id(&self) -> &'static str {
        "claude"
    }

    async fn complete(
        &self,
        req: &ProviderRequest,
        cancel: &CancellationToken,
    ) -> Result<ProviderResponse, ProviderError> {
        call_with_deadline(&req.model, cancel, self.send(req)).await
    }
}

use super::Provider;

impl Anthropic {
    async fn send(&self, req: &ProviderRequest) -> Result<ProviderResponse, ProviderError> {
        let body = build_body(req);
        let mut rb = http_ai()
            .post(self.endpoint())
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json");
        if wants_fallback(&req.model) {
            rb = rb.header("anthropic-beta", FALLBACK_BETA);
        }

        let res = rb.json(&body).send().await.map_err(|e| classify_reqwest(&e))?;
        let status = res.status().as_u16();
        let retry_after = retry_after_of(res.headers());
        let text = res.text().await.map_err(|e| classify_reqwest(&e))?;

        if !(200..300).contains(&status) {
            return Err(classify_status(status, retry_after, &text));
        }
        let json: Value = serde_json::from_str(&text)
            .map_err(|e| ProviderError::Transient(format!("תשובה לא תקינה מ-Anthropic: {}", e)))?;
        let parsed = parse_response(&json);
        if parsed.stop == StopReason::Refusal && parsed.tool_uses().is_empty() {
            return Err(ProviderError::Refusal);
        }
        Ok(parsed)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::{Role, ToolKind};

    fn fixture_tool() -> ToolSpec {
        ToolSpec {
            name: "lookup_param",
            description: "בדוק הגדרה",
            input_schema: json!({
                "type": "object",
                "properties": { "key": { "type": "string" } },
                "required": ["key"],
                "additionalProperties": false
            }),
            kind: ToolKind::ReadOnly,
        }
    }

    fn fixture_req(model: &str) -> ProviderRequest {
        ProviderRequest {
            model: model.to_string(),
            system: vec!["rules".to_string(), "catalog".to_string()],
            messages: vec![
                Message::user_text("היי"),
                Message {
                    role: Role::Assistant,
                    content: vec![
                        ContentBlock::Thinking(json!({"type":"thinking","thinking":"x","signature":"s"})),
                        ContentBlock::ToolUse {
                            id: "toolu_1".into(),
                            name: "lookup_param".into(),
                            input: json!({"key":"type"}),
                        },
                    ],
                },
                Message {
                    role: Role::User,
                    content: vec![ContentBlock::ToolResult {
                        tool_use_id: "toolu_1".into(),
                        name: "lookup_param".into(),
                        content: "type=menu".into(),
                        is_error: false,
                    }],
                },
            ],
            tools: vec![fixture_tool()],
            turn: 2,
        }
    }

    #[test]
    fn body_shape_matches_the_api() {
        let b = build_body(&fixture_req("claude-sonnet-5"));
        assert_eq!(b["max_tokens"], json!(8000));
        assert_eq!(b["thinking"]["type"], json!("adaptive"));
        assert_eq!(b["output_config"]["effort"], json!("medium"));
        assert!(b.get("temperature").is_none());
        assert!(b.get("top_p").is_none());
        assert!(b["thinking"].get("budget_tokens").is_none());
        assert_eq!(b["tools"][0]["strict"], json!(true));
        assert_eq!(b["tools"][0]["input_schema"]["additionalProperties"], json!(false));
        // cache breakpoints: last system block, last block of last message
        assert!(b["system"][0].get("cache_control").is_none());
        assert_eq!(b["system"][1]["cache_control"]["type"], json!("ephemeral"));
        assert_eq!(b["messages"][2]["content"][0]["cache_control"]["type"], json!("ephemeral"));
        assert!(b["messages"][0]["content"][0].get("cache_control").is_none());
        // thinking echoed verbatim
        assert_eq!(b["messages"][1]["content"][0]["signature"], json!("s"));
        assert_eq!(b["messages"][1]["content"][1]["type"], json!("tool_use"));
        assert_eq!(b["messages"][2]["content"][0]["type"], json!("tool_result"));
        assert!(b.get("fallbacks").is_none());
    }

    #[test]
    fn opus_opts_into_server_side_fallback() {
        let b = build_body(&fixture_req("claude-opus-5"));
        assert_eq!(b["fallbacks"], json!("default"));
        assert!(wants_fallback("claude-opus-5"));
        assert!(!wants_fallback("claude-sonnet-5"));
    }

    #[test]
    fn tools_and_system_are_byte_stable_across_builds() {
        let a = build_body(&fixture_req("claude-sonnet-5"));
        let b = build_body(&fixture_req("claude-sonnet-5"));
        assert_eq!(
            serde_json::to_string(&a["tools"]).unwrap(),
            serde_json::to_string(&b["tools"]).unwrap()
        );
        assert_eq!(
            serde_json::to_string(&a["system"]).unwrap(),
            serde_json::to_string(&b["system"]).unwrap()
        );
    }

    #[test]
    fn response_parsing() {
        let json = json!({
            "content": [
                {"type":"thinking","thinking":"t","signature":"sig"},
                {"type":"text","text":"שלום"},
                {"type":"tool_use","id":"toolu_9","name":"lookup_param","input":{"key":"type"}}
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 10, "output_tokens": 3,
                      "cache_read_input_tokens": 900, "cache_creation_input_tokens": 5}
        });
        let r = parse_response(&json);
        assert_eq!(r.stop, StopReason::ToolUse);
        assert_eq!(r.text(), "שלום");
        assert_eq!(r.tool_uses().len(), 1);
        assert!(matches!(r.content[0], ContentBlock::Thinking(_)));
        assert_eq!(r.usage.cache_read, 900);
        assert_eq!(r.usage.cache_write, 5);
        assert!(r.usage.cache_hit_pct() > 98.0);
    }

    #[test]
    fn refusal_is_detected() {
        let r = parse_response(&json!({"content":[],"stop_reason":"refusal","usage":{}}));
        assert_eq!(r.stop, StopReason::Refusal);
    }
}
