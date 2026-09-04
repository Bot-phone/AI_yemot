//! Google Gemini `generateContent` (v1beta).
//!
//! Three traps this module exists to avoid:
//!   1. `candidates[0].content.parts` may contain text AND functionCall together
//!      — every part must be walked, not just `parts[0]`.
//!   2. `functionCall` has no id, so ids are minted per turn and must round-trip
//!      through `functionResponse` **in call order**.
//!   3. `thoughtsTokenCount` is NOT part of `candidatesTokenCount`; it is added
//!      to output here or Gemini cost is under-reported.

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

const MAX_OUTPUT_TOKENS: u64 = 8192;

pub struct Gemini {
    api_key: String,
    model: String,
    base_url: String,
}

impl Gemini {
    pub fn new(api_key: String, model: String, base_url: String) -> Self {
        Gemini {
            api_key,
            model,
            base_url,
        }
    }

    fn url(&self) -> String {
        if self.base_url.is_empty() {
            format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
                self.model, self.api_key
            )
        } else if self.base_url.ends_with(":generateContent") {
            if self.base_url.contains("key=") {
                self.base_url.clone()
            } else {
                format!("{}?key={}", self.base_url, self.api_key)
            }
        } else {
            format!(
                "{}/models/{}:generateContent?key={}",
                self.base_url, self.model, self.api_key
            )
        }
    }
}

/// Id minted for a functionCall that the API returns without one.
pub fn mint_id(turn: u32, idx: usize) -> String {
    format!("gemini_{}_{}", turn, idx)
}

// ---------------------------------------------------------------------------
// Schema translation
// ---------------------------------------------------------------------------

/// JSON-Schema → Gemini `Schema`: types UPPERCASED, `additionalProperties` and
/// `strict` dropped, `["string","null"]` collapsed to STRING + `nullable`.
pub fn to_gemini_schema(schema: &Value) -> Value {
    match schema {
        Value::Object(o) => {
            let mut out = Map::new();
            let mut nullable = false;
            for (k, v) in o {
                match k.as_str() {
                    "additionalProperties" | "strict" | "$schema" | "default" | "minimum"
                    | "maximum" => continue,
                    "type" => match v {
                        Value::String(s) => {
                            out.insert("type".to_string(), json!(s.to_uppercase()));
                        }
                        Value::Array(items) => {
                            for it in items {
                                match it.as_str() {
                                    Some("null") => nullable = true,
                                    Some(t) => {
                                        out.insert("type".to_string(), json!(t.to_uppercase()));
                                    }
                                    None => {}
                                }
                            }
                        }
                        _ => {}
                    },
                    "properties" => {
                        let mut props = Map::new();
                        if let Some(p) = v.as_object() {
                            for (name, sub) in p {
                                props.insert(name.clone(), to_gemini_schema(sub));
                            }
                        }
                        out.insert("properties".to_string(), Value::Object(props));
                    }
                    "items" => {
                        out.insert("items".to_string(), to_gemini_schema(v));
                    }
                    _ => {
                        out.insert(k.clone(), v.clone());
                    }
                }
            }
            if nullable {
                out.insert("nullable".to_string(), json!(true));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

pub fn tools_json(tools: &[ToolSpec]) -> Value {
    json!([{
        "functionDeclarations": Value::Array(tools.iter().map(|t| json!({
            "name": t.name,
            "description": t.description,
            "parameters": to_gemini_schema(&t.input_schema),
        })).collect())
    }])
}

// ---------------------------------------------------------------------------
// Request mapping
// ---------------------------------------------------------------------------

pub fn contents_json(messages: &[Message]) -> Value {
    let mut out: Vec<Value> = Vec::new();
    for m in messages {
        let mut parts: Vec<Value> = Vec::new();
        let mut responses: Vec<Value> = Vec::new();

        for b in &m.content {
            match b {
                ContentBlock::Text(t) => parts.push(json!({ "text": t })),
                ContentBlock::ToolUse { name, input, .. } => parts.push(json!({
                    "functionCall": { "name": name, "args": input }
                })),
                ContentBlock::ToolResult { name, content, .. } => responses.push(json!({
                    "functionResponse": {
                        "name": name,
                        "response": { "content": content }
                    }
                })),
                ContentBlock::Thinking(_) => {}
            }
        }

        // Function responses form their own turn, in call order.
        if !responses.is_empty() {
            out.push(json!({ "role": "function", "parts": Value::Array(responses) }));
        }
        if !parts.is_empty() {
            out.push(json!({
                "role": if m.role == Role::Assistant { "model" } else { "user" },
                "parts": Value::Array(parts),
            }));
        }
    }
    Value::Array(out)
}

pub fn build_body(req: &ProviderRequest) -> Value {
    json!({
        "systemInstruction": { "parts": [{ "text": req.system.join("\n\n") }] },
        "contents": contents_json(&req.messages),
        "tools": tools_json(&req.tools),
        "generationConfig": {
            "temperature": 0,
            "maxOutputTokens": MAX_OUTPUT_TOKENS,
        },
        "safetySettings": [
            { "category": "HARM_CATEGORY_HARASSMENT", "threshold": "BLOCK_NONE" },
            { "category": "HARM_CATEGORY_HATE_SPEECH", "threshold": "BLOCK_NONE" },
            { "category": "HARM_CATEGORY_DANGEROUS_CONTENT", "threshold": "BLOCK_NONE" }
        ]
    })
}

// ---------------------------------------------------------------------------
// Response mapping
// ---------------------------------------------------------------------------

pub fn parse_response(json: &Value, turn: u32) -> ProviderResponse {
    let mut content = Vec::new();
    let mut call_idx = 0usize;

    if let Some(parts) = json["candidates"][0]["content"]["parts"].as_array() {
        for p in parts {
            if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                if !t.trim().is_empty() {
                    content.push(ContentBlock::Text(t.to_string()));
                }
            }
            if let Some(fc) = p.get("functionCall") {
                content.push(ContentBlock::ToolUse {
                    id: mint_id(turn, call_idx),
                    name: fc.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    input: fc.get("args").cloned().unwrap_or_else(|| json!({})),
                });
                call_idx += 1;
            }
        }
    }

    let finish = json["candidates"][0]["finishReason"].as_str();
    let um = json.get("usageMetadata");
    let n = |k: &str| -> u64 {
        um.and_then(|u| u.get(k)).and_then(|v| v.as_u64()).unwrap_or(0)
    };
    let cached = n("cachedContentTokenCount");

    ProviderResponse {
        content,
        stop: match finish {
            Some("STOP") | None => StopReason::EndTurn,
            Some("MAX_TOKENS") => StopReason::MaxTokens,
            Some("SAFETY") | Some("PROHIBITED_CONTENT") | Some("BLOCKLIST") => StopReason::Refusal,
            Some(o) => StopReason::Other(o.to_string()),
        },
        usage: Usage {
            input: n("promptTokenCount").saturating_sub(cached),
            // thoughtsTokenCount is billed as output but reported separately.
            output: n("candidatesTokenCount") + n("thoughtsTokenCount"),
            cache_read: cached,
            cache_write: 0,
        },
    }
}

// ---------------------------------------------------------------------------

#[async_trait]
impl Provider for Gemini {
    fn id(&self) -> &'static str {
        "gemini"
    }

    async fn complete(
        &self,
        req: &ProviderRequest,
        cancel: &CancellationToken,
    ) -> Result<ProviderResponse, ProviderError> {
        call_with_deadline(&req.model, cancel, self.send(req)).await
    }
}

impl Gemini {
    async fn send(&self, req: &ProviderRequest) -> Result<ProviderResponse, ProviderError> {
        let res = http_ai()
            .post(self.url())
            .header("Content-Type", "application/json")
            .json(&build_body(req))
            .send()
            .await
            .map_err(|e| classify_reqwest(&e))?;

        let status = res.status().as_u16();
        let retry_after = retry_after_of(res.headers());
        let text = res.text().await.map_err(|e| classify_reqwest(&e))?;
        if !(200..300).contains(&status) {
            return Err(classify_status(status, retry_after, &text));
        }
        let json: Value = serde_json::from_str(&text)
            .map_err(|e| ProviderError::Transient(format!("תשובה לא תקינה מ-Gemini: {}", e)))?;
        let parsed = parse_response(&json, req.turn);
        if parsed.stop == StopReason::Refusal && parsed.tool_uses().is_empty() {
            return Err(ProviderError::Refusal);
        }
        Ok(parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::ToolKind;

    fn tool() -> ToolSpec {
        ToolSpec {
            name: "get_knowledge_section",
            description: "קרא סעיף",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file": { "type": "string" },
                    "heading": { "type": ["string", "null"] }
                },
                "required": ["file", "heading"],
                "additionalProperties": false
            }),
            kind: ToolKind::ReadOnly,
        }
    }

    #[test]
    fn schema_is_uppercased_and_stripped() {
        let s = to_gemini_schema(&tool().input_schema);
        assert_eq!(s["type"], json!("OBJECT"));
        assert!(s.get("additionalProperties").is_none());
        assert_eq!(s["properties"]["file"]["type"], json!("STRING"));
        assert_eq!(s["properties"]["heading"]["type"], json!("STRING"));
        assert_eq!(s["properties"]["heading"]["nullable"], json!(true));
        assert_eq!(s["required"], json!(["file", "heading"]));
    }

    #[test]
    fn body_keeps_safety_settings_and_zero_temperature() {
        let req = ProviderRequest {
            model: "gemini-2.5-flash".into(),
            system: vec!["rules".into(), "catalog".into()],
            messages: vec![Message::user_text("היי")],
            tools: vec![tool()],
            turn: 1,
        };
        let b = build_body(&req);
        assert_eq!(b["generationConfig"]["temperature"], json!(0));
        assert_eq!(b["generationConfig"]["maxOutputTokens"], json!(8192));
        assert_eq!(b["safetySettings"].as_array().unwrap().len(), 3);
        assert_eq!(b["safetySettings"][2]["threshold"], json!("BLOCK_NONE"));
        assert_eq!(b["systemInstruction"]["parts"][0]["text"], json!("rules\n\ncatalog"));
        assert_eq!(b["contents"][0]["role"], json!("user"));
        assert_eq!(b["tools"][0]["functionDeclarations"][0]["name"], json!("get_knowledge_section"));
        assert!(b["tools"][0]["functionDeclarations"][0].get("strict").is_none());
    }

    #[test]
    fn function_responses_round_trip_in_call_order() {
        let msgs = vec![
            Message {
                role: Role::Assistant,
                content: vec![
                    ContentBlock::Text("בודק".into()),
                    ContentBlock::ToolUse {
                        id: mint_id(1, 0),
                        name: "a".into(),
                        input: json!({"x":1}),
                    },
                    ContentBlock::ToolUse {
                        id: mint_id(1, 1),
                        name: "b".into(),
                        input: json!({}),
                    },
                ],
            },
            Message {
                role: Role::User,
                content: vec![
                    ContentBlock::ToolResult {
                        tool_use_id: mint_id(1, 0),
                        name: "a".into(),
                        content: "ra".into(),
                        is_error: false,
                    },
                    ContentBlock::ToolResult {
                        tool_use_id: mint_id(1, 1),
                        name: "b".into(),
                        content: "rb".into(),
                        is_error: false,
                    },
                ],
            },
        ];
        let c = contents_json(&msgs);
        assert_eq!(c[0]["role"], json!("model"));
        assert_eq!(c[0]["parts"][0]["text"], json!("בודק"));
        assert_eq!(c[0]["parts"][1]["functionCall"]["name"], json!("a"));
        assert_eq!(c[1]["role"], json!("function"));
        assert_eq!(c[1]["parts"][0]["functionResponse"]["name"], json!("a"));
        assert_eq!(c[1]["parts"][0]["functionResponse"]["response"]["content"], json!("ra"));
        assert_eq!(c[1]["parts"][1]["functionResponse"]["name"], json!("b"));
        assert_eq!(mint_id(3, 2), "gemini_3_2");
    }

    #[test]
    fn all_parts_are_walked_text_plus_two_calls() {
        let json = json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": { "parts": [
                    { "text": "רגע" },
                    { "functionCall": { "name": "search_knowledge", "args": {"query":"תפריט"} } },
                    { "text": "וגם" },
                    { "functionCall": { "name": "lookup_param", "args": {"key":"type"} } }
                ]}
            }],
            "usageMetadata": {
                "promptTokenCount": 1000, "candidatesTokenCount": 20,
                "cachedContentTokenCount": 700, "thoughtsTokenCount": 15
            }
        });
        let r = parse_response(&json, 4);
        let calls = r.tool_uses();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "gemini_4_0");
        assert_eq!(calls[1].0, "gemini_4_1");
        assert_eq!(calls[1].1, "lookup_param");
        assert_eq!(r.text(), "רגע\n\nוגם");
        // finishReason STOP alongside function calls — tool calls still win.
        assert_eq!(r.stop, StopReason::EndTurn);
        assert_eq!(r.usage.input, 300);
        assert_eq!(r.usage.cache_read, 700);
        assert_eq!(r.usage.output, 35);
    }

    #[test]
    fn url_variants() {
        let g = Gemini::new("K".into(), "gemini-2.5-flash".into(), String::new());
        assert!(g.url().contains("/models/gemini-2.5-flash:generateContent?key=K"));
        let g2 = Gemini::new("K".into(), "m".into(), "https://p.io/v1beta".into());
        assert_eq!(g2.url(), "https://p.io/v1beta/models/m:generateContent?key=K");
        let g3 = Gemini::new(
            "K".into(),
            "m".into(),
            "https://p.io/v1beta/models/m:generateContent".into(),
        );
        assert!(g3.url().ends_with("?key=K"));
    }
}
