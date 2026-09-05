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
    call_with_deadline, classify_reqwest, classify_status, http_ai, retry_after_of, short, sse_data,
    DeltaBatch, DeltaSink, LineReader, Provider,
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
        self.url_for("generateContent")
    }

    /// The streaming endpoint is the same URL with a different method and
    /// `alt=sse` (without it the response is a JSON array, not an event stream).
    fn stream_url(&self) -> String {
        let u = self.url_for("streamGenerateContent");
        if u.contains("alt=sse") {
            u
        } else if u.contains('?') {
            format!("{}&alt=sse", u)
        } else {
            format!("{}?alt=sse", u)
        }
    }

    /// The API key is deliberately NOT in the query string: a transport error,
    /// a redirect or a log line would then carry the key. It travels in the
    /// `x-goog-api-key` header instead (see [`Self::authed`]).
    fn url_for(&self, method: &str) -> String {
        if self.base_url.is_empty() {
            format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:{}",
                self.model, method
            )
        } else if self.base_url.ends_with(":generateContent") {
            format!(
                "{}:{}",
                self.base_url.trim_end_matches(":generateContent"),
                method
            )
        } else {
            format!("{}/models/{}:{}", self.base_url, self.model, method)
        }
    }

    fn authed(&self, url: String) -> reqwest::RequestBuilder {
        http_ai()
            .post(url)
            .header("x-goog-api-key", &self.api_key)
            .header("Content-Type", "application/json")
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

        // Function responses form their own turn, in call order. `Content.role`
        // in the v1beta schema is only ever "user" or "model" — a
        // functionResponse comes back to the model as a user turn.
        if !responses.is_empty() {
            out.push(json!({ "role": "user", "parts": Value::Array(responses) }));
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

/// Flash bills "thoughts" as output and thinks by default; for this workload
/// (schema-driven tool calls over documented parameters) that is paid latency.
/// Pro cannot disable thinking at all, so the key is omitted there.
pub fn thinking_config(model: &str) -> Option<Value> {
    if model.to_ascii_lowercase().contains("flash") {
        Some(json!({ "thinkingBudget": 0 }))
    } else {
        None
    }
}

pub fn build_body(req: &ProviderRequest) -> Value {
    let mut generation = Map::new();
    generation.insert("temperature".to_string(), json!(0));
    generation.insert("maxOutputTokens".to_string(), json!(MAX_OUTPUT_TOKENS));
    if let Some(tc) = thinking_config(&req.model) {
        generation.insert("thinkingConfig".to_string(), tc);
    }
    json!({
        "systemInstruction": { "parts": [{ "text": req.system.join("\n\n") }] },
        "contents": contents_json(&req.messages),
        "tools": tools_json(&req.tools),
        "generationConfig": Value::Object(generation),
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
            if p.get("thought").and_then(|v| v.as_bool()) == Some(true) {
                continue;
            }
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

    ProviderResponse {
        content,
        stop: parse_stop(json["candidates"][0]["finishReason"].as_str()),
        usage: parse_usage(json.get("usageMetadata")),
    }
}

pub fn parse_stop(finish: Option<&str>) -> StopReason {
    match finish {
        Some("STOP") | None => StopReason::EndTurn,
        Some("MAX_TOKENS") => StopReason::MaxTokens,
        Some("SAFETY") | Some("PROHIBITED_CONTENT") | Some("BLOCKLIST") => StopReason::Refusal,
        Some(o) => StopReason::Other(o.to_string()),
    }
}

pub fn parse_usage(um: Option<&Value>) -> Usage {
    let n = |k: &str| -> u64 {
        um.and_then(|u| u.get(k)).and_then(|v| v.as_u64()).unwrap_or(0)
    };
    let cached = n("cachedContentTokenCount");
    Usage {
        input: n("promptTokenCount").saturating_sub(cached),
        // thoughtsTokenCount is billed as output but reported separately.
        output: n("candidatesTokenCount") + n("thoughtsTokenCount"),
        cache_read: cached,
        cache_write: 0,
    }
}

// ---------------------------------------------------------------------------
// SSE accumulator
// ---------------------------------------------------------------------------

/// `streamGenerateContent?alt=sse` sends whole `GenerateContentResponse`
/// objects, each holding the *next* slice of the candidate — text arrives
/// split across parts and a `functionCall` arrives whole, in its own chunk.
#[derive(Default)]
pub struct StreamAcc {
    text: String,
    calls: Vec<(String, Value)>,
    finish_reason: Option<String>,
    usage: Usage,
}

impl StreamAcc {
    pub fn feed_line(
        &mut self,
        line: &str,
        deltas: &mut DeltaBatch<'_>,
    ) -> Result<(), ProviderError> {
        let Some(data) = sse_data(line) else {
            return Ok(());
        };
        let Ok(chunk) = serde_json::from_str::<Value>(data) else {
            return Ok(());
        };
        if let Some(err) = chunk.get("error") {
            // Not `BadRequest`: that verdict belongs to the HTTP status, where
            // it means "this endpoint has no SSE, retry unstreamed".
            return Err(ProviderError::Transient(short(&err.to_string())));
        }
        if let Some(parts) = chunk["candidates"][0]["content"]["parts"].as_array() {
            for p in parts {
                if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                    // A thought summary is not the answer; never stream it.
                    if p.get("thought").and_then(|v| v.as_bool()) != Some(true) {
                        self.text.push_str(t);
                        deltas.push(t);
                    }
                }
                if let Some(fc) = p.get("functionCall") {
                    self.calls.push((
                        fc.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        fc.get("args").cloned().unwrap_or_else(|| json!({})),
                    ));
                }
            }
        }
        if let Some(f) = chunk["candidates"][0]["finishReason"].as_str() {
            self.finish_reason = Some(f.to_string());
        }
        // Every chunk restates the totals; the last one wins.
        if let Some(um) = chunk.get("usageMetadata") {
            if !um.is_null() {
                self.usage = parse_usage(Some(um));
            }
        }
        Ok(())
    }

    pub fn has_content(&self) -> bool {
        !self.text.trim().is_empty() || !self.calls.is_empty()
    }

    pub fn finish(self, turn: u32, aborted: bool) -> ProviderResponse {
        let mut content = Vec::new();
        if !self.text.trim().is_empty() {
            content.push(ContentBlock::Text(self.text));
        }
        for (i, (name, args)) in self.calls.into_iter().enumerate() {
            content.push(ContentBlock::ToolUse {
                id: mint_id(turn, i),
                name,
                input: args,
            });
        }
        let stop = if aborted && self.finish_reason.is_none() {
            StopReason::Other("stream_incomplete".to_string())
        } else {
            parse_stop(self.finish_reason.as_deref())
        };
        ProviderResponse {
            content,
            stop,
            usage: self.usage,
        }
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
        on_delta: DeltaSink<'_>,
    ) -> Result<ProviderResponse, ProviderError> {
        call_with_deadline(&req.model, cancel, self.send(req, cancel, on_delta)).await
    }
}

impl Gemini {
    /// Stream first; a 4xx from `streamGenerateContent` means the endpoint does
    /// not offer it, so the turn is retried once against `generateContent`.
    async fn send(
        &self,
        req: &ProviderRequest,
        cancel: &CancellationToken,
        on_delta: DeltaSink<'_>,
    ) -> Result<ProviderResponse, ProviderError> {
        match self.send_stream(req, cancel, on_delta).await {
            Err(ProviderError::BadRequest(_)) => self.send_once(req).await,
            other => other,
        }
    }

    async fn send_stream(
        &self,
        req: &ProviderRequest,
        cancel: &CancellationToken,
        on_delta: DeltaSink<'_>,
    ) -> Result<ProviderResponse, ProviderError> {
        let res = self
            .authed(self.stream_url())
            .header("Accept", "text/event-stream")
            .json(&build_body(req))
            .send()
            .await
            .map_err(classify_reqwest)?;
        let status = res.status().as_u16();
        let retry_after = retry_after_of(res.headers());
        if !(200..300).contains(&status) {
            let text = res.text().await.unwrap_or_default();
            return Err(classify_status(status, retry_after, &text));
        }

        let mut lines = LineReader::new(res.bytes_stream());
        let mut acc = StreamAcc::default();
        let mut deltas = DeltaBatch::new(on_delta);
        loop {
            match lines.next_line(cancel).await {
                Ok(Some(line)) => {
                    if let Err(e) = acc.feed_line(&line, &mut deltas) {
                        deltas.flush();
                        if acc.has_content() {
                            break;
                        }
                        return Err(e);
                    }
                }
                Ok(None) => break,
                Err(ProviderError::Cancelled) => return Err(ProviderError::Cancelled),
                Err(e) => {
                    deltas.flush();
                    if acc.has_content() {
                        break;
                    }
                    return Err(e);
                }
            }
        }
        deltas.flush();
        let aborted = acc.finish_reason.is_none();
        let has_content = acc.has_content();
        let parsed = acc.finish(req.turn, aborted);
        if !has_content && aborted {
            // A 200 that yielded no events at all is a gateway that ignored
            // `stream`: fall back to the plain request rather than retrying.
            return Err(ProviderError::BadRequest(
                "השידור מ-Gemini הסתיים ללא תוכן".to_string(),
            ));
        }
        if parsed.stop == StopReason::Refusal && parsed.tool_uses().is_empty() {
            return Err(ProviderError::Refusal);
        }
        Ok(parsed)
    }

    async fn send_once(&self, req: &ProviderRequest) -> Result<ProviderResponse, ProviderError> {
        let res = self
            .authed(self.url())
            .json(&build_body(req))
            .send()
            .await
            .map_err(classify_reqwest)?;

        let status = res.status().as_u16();
        let retry_after = retry_after_of(res.headers());
        let text = res.text().await.map_err(classify_reqwest)?;
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

    fn req_for(model: &str) -> ProviderRequest {
        ProviderRequest {
            model: model.into(),
            system: vec!["rules".into(), "catalog".into()],
            messages: vec![Message::user_text("היי")],
            tools: vec![tool()],
            turn: 1,
        }
    }

    #[test]
    fn flash_disables_thinking_pro_does_not() {
        let flash = build_body(&req_for("gemini-2.5-flash"));
        assert_eq!(flash["generationConfig"]["thinkingConfig"]["thinkingBudget"], json!(0));
        let pro = build_body(&req_for("gemini-2.5-pro"));
        assert!(pro["generationConfig"].get("thinkingConfig").is_none());
        // temperature and the cap survive the rewrite of generationConfig
        assert_eq!(pro["generationConfig"]["temperature"], json!(0));
        assert_eq!(pro["generationConfig"]["maxOutputTokens"], json!(8192));
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
        // v1beta Content.role is only "user" / "model" — never "function".
        assert_eq!(c[1]["role"], json!("user"));
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
        assert!(g.url().ends_with("/models/gemini-2.5-flash:generateContent"));
        let g2 = Gemini::new("K".into(), "m".into(), "https://p.io/v1beta".into());
        assert_eq!(g2.url(), "https://p.io/v1beta/models/m:generateContent");
        let g3 = Gemini::new(
            "K".into(),
            "m".into(),
            "https://p.io/v1beta/models/m:generateContent".into(),
        );
        assert_eq!(g3.url(), "https://p.io/v1beta/models/m:generateContent");
    }

    /// The key must never reach a URL: URLs land in error strings, logs and
    /// redirects. It travels in `x-goog-api-key`.
    #[test]
    fn no_gemini_url_ever_carries_the_api_key() {
        for base in [
            "",
            "https://p.io/v1beta",
            "https://p.io/v1beta/models/m:generateContent",
        ] {
            let g = Gemini::new("SECRETKEY".into(), "m".into(), base.into());
            for u in [g.url(), g.stream_url()] {
                assert!(!u.contains("key="), "{} leaks the key: {}", base, u);
                assert!(!u.contains("SECRETKEY"), "{} leaks the key: {}", base, u);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Streaming
    // -----------------------------------------------------------------------

    #[test]
    fn stream_url_switches_the_method_and_asks_for_sse() {
        let g = Gemini::new("K".into(), "gemini-2.5-flash".into(), String::new());
        let u = g.stream_url();
        assert!(u.contains(":streamGenerateContent"));
        assert!(!u.contains(":generateContent?"));
        assert!(u.contains("alt=sse"));
        // the non-streaming URL is untouched
        assert!(g.url().ends_with(":generateContent"));

        let g2 = Gemini::new("K".into(), "m".into(), "https://p.io/v1beta".into());
        assert_eq!(
            g2.stream_url(),
            "https://p.io/v1beta/models/m:streamGenerateContent?alt=sse"
        );
    }

    fn run_stream(sse: &str, turn: u32) -> (ProviderResponse, String) {
        let seen = std::sync::Mutex::new(String::new());
        let sink = |d: &str| seen.lock().unwrap().push_str(d);
        let mut acc = StreamAcc::default();
        {
            let mut deltas = DeltaBatch::new(&sink);
            for line in sse.split('\n') {
                acc.feed_line(line.trim_end_matches('\r'), &mut deltas).unwrap();
            }
        }
        let aborted = acc.finish_reason.is_none();
        let out = acc.finish(turn, aborted);
        let text = seen.lock().unwrap().clone();
        (out, text)
    }

    #[test]
    fn stream_accumulates_text_then_a_function_call() {
        let sse = concat!(
            r#"data: {"candidates":[{"content":{"parts":[{"text":"בודק"}],"role":"model"}}]}"#,
            "\n\n",
            r#"data: {"candidates":[{"content":{"parts":[{"text":" את השלוחה"}],"role":"model"}}]}"#,
            "\n\n",
            r#"data: {"candidates":[{"content":{"parts":[{"functionCall":{"name":"lookup_param","args":{"key":"type"}}}],"role":"model"}}]}"#,
            "\n\n",
            r#"data: {"candidates":[{"content":{"parts":[]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":1000,"candidatesTokenCount":30,"thoughtsTokenCount":5,"cachedContentTokenCount":700}}"#,
            "\n\n",
        );
        let (r, streamed) = run_stream(sse, 4);
        assert_eq!(streamed, "בודק את השלוחה");
        assert_eq!(r.text(), "בודק את השלוחה");
        let calls = r.tool_uses();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "gemini_4_0");
        assert_eq!(calls[0].1, "lookup_param");
        assert_eq!(calls[0].2["key"], json!("type"));
        // finishReason STOP alongside a call — the runner still sees the call
        assert_eq!(r.stop, StopReason::EndTurn);
        // usageMetadata of the LAST chunk, thoughts folded into output
        assert_eq!(r.usage.input, 300);
        assert_eq!(r.usage.cache_read, 700);
        assert_eq!(r.usage.output, 35);
    }

    #[test]
    fn a_thought_part_is_not_streamed_to_the_ui() {
        let sse = concat!(
            r#"data: {"candidates":[{"content":{"parts":[{"text":"מחשבה","thought":true}]}}]}"#,
            "\n",
            r#"data: {"candidates":[{"content":{"parts":[{"text":"תשובה"}]},"finishReason":"STOP"}]}"#,
            "\n",
        );
        let (r, streamed) = run_stream(sse, 1);
        assert_eq!(streamed, "תשובה");
        assert_eq!(r.text(), "תשובה");
    }

    #[test]
    fn a_truncated_gemini_stream_keeps_what_arrived() {
        let sse = concat!(
            r#"data: {"candidates":[{"content":{"parts":[{"text":"חצי"}]}}]}"#,
            "\n",
            r#"data: {"candidates":[{"content":{"par"#,
        );
        let (r, streamed) = run_stream(sse, 2);
        assert_eq!(streamed, "חצי");
        assert_eq!(r.text(), "חצי");
        assert_eq!(r.stop, StopReason::Other("stream_incomplete".to_string()));
    }

    #[test]
    fn a_gemini_error_frame_is_transient() {
        let seen = std::sync::Mutex::new(String::new());
        let sink = |d: &str| seen.lock().unwrap().push_str(d);
        let mut deltas = DeltaBatch::new(&sink);
        let mut acc = StreamAcc::default();
        let e = acc
            .feed_line(
                r#"data: {"error":{"code":500,"message":"internal","status":"INTERNAL"}}"#,
                &mut deltas,
            )
            .unwrap_err();
        assert!(matches!(e, ProviderError::Transient(_)));
        assert!(e.retryable());
        assert!(!acc.has_content());
    }
}
