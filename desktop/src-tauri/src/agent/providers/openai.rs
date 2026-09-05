//! OpenAI-compatible chat completions: `openai`, `groq` and any `custom`
//! endpoint the user points at (the URL/model resolution is the same logic the
//! old single-shot `ai.rs` path used).

use async_trait::async_trait;
use serde_json::{json, Map, Value};
use tokio_util::sync::CancellationToken;

use super::super::types::{
    ContentBlock, Message, ProviderError, ProviderRequest, ProviderResponse, Role, StopReason,
    ToolSpec, Usage,
};
use super::{
    call_with_deadline, classify_reqwest, classify_status, http_ai, retry_after_of, Provider,
};

const MAX_TOKENS: u64 = 8000;

/// Stable across every turn of every run so OpenAI routes the request to the
/// machine that already holds this prefix in its cache.
const PROMPT_CACHE_KEY: &str = "ai-yemot-agent";

/// Marker input handed to the dispatcher when `arguments` was not valid JSON.
pub const INVALID_ARGUMENTS_KEY: &str = "__invalid_arguments";

pub struct OpenAiCompatible {
    api_key: String,
    url: String,
    flavour: String,
}

impl OpenAiCompatible {
    /// The model is not stored: `ProviderRequest::model` is authoritative.
    pub fn new(api_key: String, base_url: String, flavour: &str) -> Self {
        let default_url = match flavour {
            "groq" => "https://api.groq.com/openai/v1/chat/completions",
            _ => "https://api.openai.com/v1/chat/completions",
        };
        let url = if base_url.is_empty() {
            default_url.to_string()
        } else if base_url.ends_with("/chat/completions") {
            base_url
        } else {
            format!("{}/chat/completions", base_url)
        };
        OpenAiCompatible {
            api_key,
            url,
            flavour: flavour.to_string(),
        }
    }
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
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                        "strict": true,
                    }
                })
            })
            .collect(),
    )
}

/// System first, then the conversation. Tool results become one `role:"tool"`
/// message per call — they can never share a message with anything else.
pub fn messages_json(system: &[String], messages: &[Message]) -> Value {
    let mut out: Vec<Value> = vec![json!({
        "role": "system",
        "content": system.join("\n\n"),
    })];

    for m in messages {
        match m.role {
            Role::Assistant => {
                let mut text = String::new();
                let mut calls: Vec<Value> = Vec::new();
                for b in &m.content {
                    match b {
                        ContentBlock::Text(t) => {
                            if !text.is_empty() {
                                text.push_str("\n\n");
                            }
                            text.push_str(t);
                        }
                        ContentBlock::ToolUse { id, name, input } => calls.push(json!({
                            "id": id,
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": serde_json::to_string(input)
                                    .unwrap_or_else(|_| "{}".to_string()),
                            }
                        })),
                        _ => {}
                    }
                }
                let mut o = Map::new();
                o.insert("role".to_string(), json!("assistant"));
                o.insert(
                    "content".to_string(),
                    if text.is_empty() { Value::Null } else { json!(text) },
                );
                if !calls.is_empty() {
                    o.insert("tool_calls".to_string(), Value::Array(calls));
                }
                out.push(Value::Object(o));
            }
            Role::User => {
                let mut text = String::new();
                for b in &m.content {
                    match b {
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } => out.push(json!({
                            "role": "tool",
                            "tool_call_id": tool_use_id,
                            "content": content,
                        })),
                        ContentBlock::Text(t) => {
                            if !text.is_empty() {
                                text.push_str("\n\n");
                            }
                            text.push_str(t);
                        }
                        _ => {}
                    }
                }
                if !text.is_empty() {
                    out.push(json!({ "role": "user", "content": text }));
                }
            }
        }
    }
    Value::Array(out)
}

/// Reasoning models (o-series, GPT-5) rejected `max_tokens` in favour of
/// `max_completion_tokens`, and accept only the default temperature.
pub fn is_reasoning_model(model: &str) -> bool {
    let m = model.trim().to_ascii_lowercase();
    ["o1", "o3", "o4", "gpt-5"].iter().any(|p| m.starts_with(p))
}

pub fn build_body(req: &ProviderRequest) -> Value {
    let mut body = Map::new();
    body.insert("model".to_string(), json!(req.model));
    body.insert(
        "messages".to_string(),
        messages_json(&req.system, &req.messages),
    );
    body.insert("tools".to_string(), tools_json(&req.tools));
    body.insert("prompt_cache_key".to_string(), json!(PROMPT_CACHE_KEY));
    if is_reasoning_model(&req.model) {
        body.insert("max_completion_tokens".to_string(), json!(MAX_TOKENS));
    } else {
        body.insert("temperature".to_string(), json!(0));
        body.insert("max_tokens".to_string(), json!(MAX_TOKENS));
    }
    Value::Object(body)
}

// ---------------------------------------------------------------------------
// Response mapping
// ---------------------------------------------------------------------------

pub fn parse_response(json: &Value) -> ProviderResponse {
    let msg = &json["choices"][0]["message"];
    let mut content = Vec::new();

    if let Some(t) = msg.get("content").and_then(|v| v.as_str()) {
        if !t.trim().is_empty() {
            content.push(ContentBlock::Text(t.to_string()));
        }
    }

    if let Some(calls) = msg.get("tool_calls").and_then(|v| v.as_array()) {
        for (i, c) in calls.iter().enumerate() {
            let f = &c["function"];
            let name = f.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let id = c
                .get("id")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| format!("call_{}", i));
            let raw = f.get("arguments").and_then(|v| v.as_str()).unwrap_or("");
            // `arguments` is a JSON *string*. A model that emits broken JSON must
            // get an is_error tool result back, not crash the run.
            let input = if raw.trim().is_empty() {
                json!({})
            } else {
                match serde_json::from_str::<Value>(raw) {
                    Ok(v) if v.is_object() => v,
                    _ => json!({ INVALID_ARGUMENTS_KEY: raw }),
                }
            };
            content.push(ContentBlock::ToolUse { id, name, input });
        }
    }

    let finish = json["choices"][0]["finish_reason"].as_str();
    let u = json.get("usage");
    let cached = u
        .and_then(|u| u.get("prompt_tokens_details"))
        .and_then(|d| d.get("cached_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let prompt = u
        .and_then(|u| u.get("prompt_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    ProviderResponse {
        content,
        stop: match finish {
            Some("tool_calls") | Some("function_call") => StopReason::ToolUse,
            Some("length") => StopReason::MaxTokens,
            Some("content_filter") => StopReason::Refusal,
            Some("stop") | None => StopReason::EndTurn,
            Some(o) => StopReason::Other(o.to_string()),
        },
        usage: Usage {
            // `prompt_tokens` includes the cached part; report the fresh remainder.
            input: prompt.saturating_sub(cached),
            output: u
                .and_then(|u| u.get("completion_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
            cache_read: cached,
            cache_write: 0,
        },
    }
}

// ---------------------------------------------------------------------------

#[async_trait]
impl Provider for OpenAiCompatible {
    fn id(&self) -> &'static str {
        "openai"
    }

    async fn complete(
        &self,
        req: &ProviderRequest,
        cancel: &CancellationToken,
    ) -> Result<ProviderResponse, ProviderError> {
        call_with_deadline(&req.model, cancel, self.send(req)).await
    }
}

impl OpenAiCompatible {
    /// The provider label shown to the user (a custom endpoint shows its host).
    pub fn display(&self) -> String {
        if self.flavour == "custom" {
            reqwest::Url::parse(&self.url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_string))
                .unwrap_or_else(|| "ספק מותאם".to_string())
        } else {
            self.flavour.clone()
        }
    }

    async fn send(&self, req: &ProviderRequest) -> Result<ProviderResponse, ProviderError> {
        let body = build_body(req);
        let res = http_ai()
            .post(&self.url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| classify_reqwest(&e))?;

        let status = res.status().as_u16();
        let retry_after = retry_after_of(res.headers());
        let text = res.text().await.map_err(|e| classify_reqwest(&e))?;
        if !(200..300).contains(&status) {
            return Err(classify_status(status, retry_after, &text));
        }
        let json: Value = serde_json::from_str(&text).map_err(|e| {
            ProviderError::Transient(format!("תשובה לא תקינה מ-{}: {}", self.display(), e))
        })?;
        Ok(parse_response(&json))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::ToolKind;

    fn tool() -> ToolSpec {
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

    fn req() -> ProviderRequest {
        ProviderRequest {
            model: "gpt-4.1-mini".to_string(),
            system: vec!["rules".into(), "catalog".into()],
            messages: vec![
                Message::user_text("היי"),
                Message {
                    role: Role::Assistant,
                    content: vec![
                        ContentBlock::Text("בודק".into()),
                        ContentBlock::ToolUse {
                            id: "call_1".into(),
                            name: "lookup_param".into(),
                            input: json!({"key":"type"}),
                        },
                    ],
                },
                Message {
                    role: Role::User,
                    content: vec![ContentBlock::ToolResult {
                        tool_use_id: "call_1".into(),
                        name: "lookup_param".into(),
                        content: "type=menu".into(),
                        is_error: false,
                    }],
                },
            ],
            tools: vec![tool()],
            turn: 2,
        }
    }

    #[test]
    fn body_shape() {
        let b = build_body(&req());
        assert_eq!(b["temperature"], json!(0));
        assert_eq!(b["max_tokens"], json!(8000));
        assert_eq!(b["prompt_cache_key"], json!("ai-yemot-agent"));
        assert!(b.get("max_completion_tokens").is_none());
        assert_eq!(b["tools"][0]["type"], json!("function"));
        assert_eq!(b["tools"][0]["function"]["strict"], json!(true));
        assert_eq!(b["tools"][0]["function"]["name"], json!("lookup_param"));
        let m = b["messages"].as_array().unwrap();
        assert_eq!(m[0]["role"], json!("system"));
        assert_eq!(m[0]["content"], json!("rules\n\ncatalog"));
        assert_eq!(m[1]["role"], json!("user"));
        assert_eq!(m[2]["role"], json!("assistant"));
        // arguments is a JSON *string*
        assert_eq!(
            m[2]["tool_calls"][0]["function"]["arguments"],
            json!("{\"key\":\"type\"}")
        );
        assert_eq!(m[3]["role"], json!("tool"));
        assert_eq!(m[3]["tool_call_id"], json!("call_1"));
        assert_eq!(m[3]["content"], json!("type=menu"));
    }

    #[test]
    fn reasoning_models_swap_the_token_cap_and_drop_temperature() {
        for m in ["o1-mini", "o3", "o4-mini", "gpt-5", "gpt-5.1-2026-01-01"] {
            let mut r = req();
            r.model = m.to_string();
            let b = build_body(&r);
            assert_eq!(b["max_completion_tokens"], json!(8000), "{}", m);
            assert!(b.get("max_tokens").is_none(), "{}", m);
            assert!(b.get("temperature").is_none(), "{}", m);
            assert_eq!(b["prompt_cache_key"], json!("ai-yemot-agent"), "{}", m);
            assert!(is_reasoning_model(m));
        }
        for m in ["gpt-4.1-mini", "gpt-4o", "llama-3.3-70b-versatile"] {
            assert!(!is_reasoning_model(m), "{}", m);
        }
    }

    #[test]
    fn the_cache_key_is_stable_across_turns() {
        let a = build_body(&req());
        let mut later = req();
        later.turn = 9;
        let b = build_body(&later);
        assert_eq!(a["prompt_cache_key"], b["prompt_cache_key"]);
    }

    #[test]
    fn arguments_string_is_parsed() {
        let r = parse_response(&json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {"content": null, "tool_calls": [
                    {"id":"c1","type":"function","function":{"name":"lookup_param","arguments":"{\"key\":\"type\"}"}}
                ]}
            }],
            "usage": {"prompt_tokens": 100, "completion_tokens": 7,
                      "prompt_tokens_details": {"cached_tokens": 80}}
        }));
        assert_eq!(r.stop, StopReason::ToolUse);
        let calls = r.tool_uses();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].2["key"], json!("type"));
        assert_eq!(r.usage.input, 20);
        assert_eq!(r.usage.cache_read, 80);
        assert_eq!(r.usage.output, 7);
    }

    #[test]
    fn broken_arguments_are_flagged_not_fatal() {
        let r = parse_response(&json!({
            "choices": [{"finish_reason":"tool_calls","message":{"tool_calls":[
                {"id":"c1","function":{"name":"lookup_param","arguments":"{key: type"}}
            ]}}]
        }));
        let calls = r.tool_uses();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].2[INVALID_ARGUMENTS_KEY], json!("{key: type"));
    }

    #[test]
    fn text_and_tool_calls_coexist() {
        let r = parse_response(&json!({
            "choices": [{"finish_reason":"stop","message":{"content":"שלום","tool_calls":[
                {"id":"c1","function":{"name":"lookup_param","arguments":"{}"}}
            ]}}]
        }));
        // finish_reason says "stop" but there IS a tool call — never branch on stop first.
        assert_eq!(r.text(), "שלום");
        assert_eq!(r.tool_uses().len(), 1);
    }

    #[test]
    fn url_resolution() {
        let a = OpenAiCompatible::new("k".into(), String::new(), "groq");
        assert!(a.url.starts_with("https://api.groq.com"));
        let b = OpenAiCompatible::new("k".into(), "https://x.io/v1".into(), "custom");
        assert_eq!(b.url, "https://x.io/v1/chat/completions");
        let c = OpenAiCompatible::new(
            "k".into(),
            "https://x.io/v1/chat/completions".into(),
            "custom",
        );
        assert_eq!(c.url, "https://x.io/v1/chat/completions");
        assert_eq!(c.display(), "x.io");
    }
}
