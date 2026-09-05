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
    call_with_deadline, classify_reqwest, classify_status, http_ai, retry_after_of, short, sse_data,
    DeltaBatch, DeltaSink, LineReader, Provider,
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
    // Gateways (OpenRouter, LiteLLM, …) prefix the id with the vendor:
    // `openai/o3-mini` is the same reasoning model as `o3-mini`.
    let m = m.rsplit('/').next().unwrap_or(m.as_str());
    ["o1", "o3", "o4", "gpt-5"].iter().any(|p| m.starts_with(p))
}

pub fn build_body(req: &ProviderRequest, stream: bool) -> Value {
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
    if stream {
        body.insert("stream".to_string(), json!(true));
        // Without this the streamed response carries no usage at all.
        body.insert(
            "stream_options".to_string(),
            json!({ "include_usage": true }),
        );
    }
    Value::Object(body)
}

// ---------------------------------------------------------------------------
// Response mapping
// ---------------------------------------------------------------------------

/// `arguments` is a JSON *string*. A model that emits broken JSON must get an
/// `is_error` tool result back, not crash the run.
pub fn tool_input(raw: &str) -> Value {
    if raw.trim().is_empty() {
        return json!({});
    }
    match serde_json::from_str::<Value>(raw) {
        Ok(v) if v.is_object() => v,
        _ => json!({ INVALID_ARGUMENTS_KEY: raw }),
    }
}

pub fn parse_stop(finish: Option<&str>) -> StopReason {
    match finish {
        Some("tool_calls") | Some("function_call") => StopReason::ToolUse,
        Some("length") => StopReason::MaxTokens,
        Some("content_filter") => StopReason::Refusal,
        Some("stop") | None => StopReason::EndTurn,
        Some(o) => StopReason::Other(o.to_string()),
    }
}

/// `usage` has the same shape streamed and unstreamed. `prompt_tokens` includes
/// the cached part, so the fresh remainder is what gets reported as input.
pub fn parse_usage(u: Option<&Value>) -> Usage {
    let cached = u
        .and_then(|u| u.get("prompt_tokens_details"))
        .and_then(|d| d.get("cached_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let prompt = u
        .and_then(|u| u.get("prompt_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    Usage {
        input: prompt.saturating_sub(cached),
        output: u
            .and_then(|u| u.get("completion_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        cache_read: cached,
        cache_write: 0,
    }
}

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
            content.push(ContentBlock::ToolUse {
                id,
                name,
                input: tool_input(raw),
            });
        }
    }

    ProviderResponse {
        content,
        stop: parse_stop(json["choices"][0]["finish_reason"].as_str()),
        usage: parse_usage(json.get("usage")),
    }
}

// ---------------------------------------------------------------------------
// SSE accumulator
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct StreamCall {
    id: String,
    name: String,
    args: String,
}

/// Rebuilds one chat completion out of `chat.completion.chunk` frames. Pure —
/// `feed_line` is the only input, so the fixtures below need no network.
#[derive(Default)]
pub struct StreamAcc {
    text: String,
    /// Tool calls keyed by the `index` the chunks carry; they interleave.
    calls: Vec<StreamCall>,
    finish_reason: Option<String>,
    usage: Usage,
}

impl StreamAcc {
    fn call(&mut self, index: usize) -> &mut StreamCall {
        while self.calls.len() <= index {
            self.calls.push(StreamCall::default());
        }
        &mut self.calls[index]
    }

    /// Returns `true` once `[DONE]` has been seen.
    pub fn feed_line(
        &mut self,
        line: &str,
        deltas: &mut DeltaBatch<'_>,
    ) -> Result<bool, ProviderError> {
        let Some(data) = sse_data(line) else {
            return Ok(false);
        };
        if data.trim() == "[DONE]" {
            return Ok(true);
        }
        let Ok(chunk) = serde_json::from_str::<Value>(data) else {
            // A frame cut in half by a dropped connection.
            return Ok(false);
        };
        if let Some(err) = chunk.get("error") {
            // Never `BadRequest`: that verdict is reserved for the HTTP status,
            // where it is the signal to fall back to a non-streaming request.
            return Err(ProviderError::Transient(short(&err.to_string())));
        }

        // The usage-only frame at the end carries an empty `choices` array.
        if let Some(u) = chunk.get("usage") {
            if !u.is_null() {
                self.usage = parse_usage(Some(u));
            }
        }

        let choice = &chunk["choices"][0];
        if let Some(f) = choice.get("finish_reason").and_then(|v| v.as_str()) {
            self.finish_reason = Some(f.to_string());
        }
        let delta = &choice["delta"];
        if let Some(t) = delta.get("content").and_then(|v| v.as_str()) {
            self.text.push_str(t);
            deltas.push(t);
        }
        if let Some(calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
            for (nth, c) in calls.iter().enumerate() {
                let index = c.get("index").and_then(|v| v.as_u64()).unwrap_or(nth as u64) as usize;
                let id = c.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let name = c["function"]
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let args = c["function"]
                    .get("arguments")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let slot = self.call(index);
                // id and name arrive once, on the opening frame.
                if !id.is_empty() {
                    slot.id = id;
                }
                if !name.is_empty() {
                    slot.name = name;
                }
                slot.args.push_str(&args);
            }
        }
        Ok(false)
    }

    pub fn has_content(&self) -> bool {
        !self.text.is_empty() || !self.calls.is_empty()
    }

    /// `aborted` = no `[DONE]` and no `finish_reason`.
    pub fn finish(self, aborted: bool) -> ProviderResponse {
        let mut content = Vec::new();
        if !self.text.trim().is_empty() {
            content.push(ContentBlock::Text(self.text));
        }
        for (i, c) in self.calls.iter().enumerate() {
            if c.name.is_empty() {
                continue;
            }
            content.push(ContentBlock::ToolUse {
                id: if c.id.is_empty() {
                    format!("call_{}", i)
                } else {
                    c.id.clone()
                },
                name: c.name.clone(),
                input: tool_input(&c.args),
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
impl Provider for OpenAiCompatible {
    fn id(&self) -> &'static str {
        "openai"
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

    fn request(&self, req: &ProviderRequest, stream: bool) -> reqwest::RequestBuilder {
        let mut rb = http_ai()
            .post(&self.url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json");
        if stream {
            rb = rb.header("Accept", "text/event-stream");
        }
        rb.json(&build_body(req, stream))
    }

    /// Stream first. A 4xx is how a custom OpenAI-compatible gateway that does
    /// not implement SSE answers, so the turn is retried once unstreamed.
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
            .request(req, true)
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
        let mut done = false;
        loop {
            match lines.next_line(cancel).await {
                Ok(Some(line)) => match acc.feed_line(&line, &mut deltas) {
                    Ok(true) => {
                        done = true;
                        break;
                    }
                    Ok(false) => {}
                    Err(e) => {
                        deltas.flush();
                        if acc.has_content() {
                            break;
                        }
                        return Err(e);
                    }
                },
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
        let aborted = !done;
        let has_content = acc.has_content();
        let parsed = acc.finish(aborted);
        if !has_content && aborted {
            // A 200 that yielded no events at all is a gateway that ignored
            // `stream`: fall back to the plain request rather than retrying.
            return Err(ProviderError::BadRequest(format!(
                "השידור מ-{} הסתיים ללא תוכן",
                self.display()
            )));
        }
        Ok(parsed)
    }

    async fn send_once(&self, req: &ProviderRequest) -> Result<ProviderResponse, ProviderError> {
        let res = self
            .request(req, false)
            .send()
            .await
            .map_err(classify_reqwest)?;

        let status = res.status().as_u16();
        let retry_after = retry_after_of(res.headers());
        let text = res.text().await.map_err(classify_reqwest)?;
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
        let b = build_body(&req(), false);
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
            let b = build_body(&r, false);
            assert_eq!(b["max_completion_tokens"], json!(8000), "{}", m);
            assert!(b.get("max_tokens").is_none(), "{}", m);
            assert!(b.get("temperature").is_none(), "{}", m);
            assert_eq!(b["prompt_cache_key"], json!("ai-yemot-agent"), "{}", m);
            assert!(is_reasoning_model(m));
        }
        for m in ["gpt-4.1-mini", "gpt-4o", "llama-3.3-70b-versatile"] {
            assert!(!is_reasoning_model(m), "{}", m);
        }
        // a gateway prefixes the vendor; the model behind it is unchanged
        for m in ["openai/o3-mini", "azure/openai/gpt-5", "OpenAI/O4-Mini"] {
            assert!(is_reasoning_model(m), "{}", m);
        }
        assert!(!is_reasoning_model("openai/gpt-4o"));
    }

    #[test]
    fn the_cache_key_is_stable_across_turns() {
        let a = build_body(&req(), false);
        let mut later = req();
        later.turn = 9;
        let b = build_body(&later, false);
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

    // -----------------------------------------------------------------------
    // Streaming
    // -----------------------------------------------------------------------

    /// Feeds a fixture through the accumulator; returns the response and every
    /// delta the sink saw, concatenated.
    fn run_stream(sse: &str) -> (ProviderResponse, String, bool) {
        let seen = std::sync::Mutex::new(String::new());
        let sink = |d: &str| seen.lock().unwrap().push_str(d);
        let mut acc = StreamAcc::default();
        let mut done = false;
        {
            let mut deltas = DeltaBatch::new(&sink);
            for line in sse.split('\n') {
                if acc.feed_line(line.trim_end_matches('\r'), &mut deltas).unwrap() {
                    done = true;
                    break;
                }
            }
        }
        let out = acc.finish(!done);
        let text = seen.lock().unwrap().clone();
        (out, text, done)
    }

    #[test]
    fn streaming_body_asks_for_usage() {
        let b = build_body(&req(), true);
        assert_eq!(b["stream"], json!(true));
        assert_eq!(b["stream_options"]["include_usage"], json!(true));
        let n = build_body(&req(), false);
        assert!(n.get("stream").is_none());
        assert!(n.get("stream_options").is_none());
        assert_eq!(n["prompt_cache_key"], b["prompt_cache_key"]);
    }

    const TWO_CALLS: &str = concat!(
        r#"data: {"choices":[{"index":0,"delta":{"role":"assistant","content":"בודק"}}]}"#,
        "\n\n",
        r#"data: {"choices":[{"index":0,"delta":{"content":" עכשיו"}}]}"#,
        "\n",
        r#"data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"lookup_param","arguments":""}}]}}]}"#,
        "\n",
        r#"data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"id":"call_b","function":{"name":"lookup_param","arguments":""}}]}}]}"#,
        "\n",
        // the two calls' arguments now interleave, chunk by chunk
        r#"data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"key\":"}}]}}]}"#,
        "\n",
        r#"data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"function":{"arguments":"{\"key\":"}}]}}]}"#,
        "\n",
        r#"data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"type\"}"}}]}}]}"#,
        "\n",
        r#"data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"function":{"arguments":"\"ext\"}"}}]}}]}"#,
        "\n",
        r#"data: {"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
        "\n",
        r#"data: {"choices":[],"usage":{"prompt_tokens":1000,"completion_tokens":60,"prompt_tokens_details":{"cached_tokens":900}}}"#,
        "\n",
        "data: [DONE]\n",
    );

    #[test]
    fn stream_reassembles_two_interleaved_tool_calls() {
        let (r, streamed, done) = run_stream(TWO_CALLS);
        assert!(done);
        assert_eq!(streamed, "בודק עכשיו");
        assert_eq!(r.text(), "בודק עכשיו");
        assert_eq!(r.stop, StopReason::ToolUse);

        let calls = r.tool_uses();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "call_a");
        assert_eq!(calls[0].2["key"], json!("type"));
        assert_eq!(calls[1].0, "call_b");
        assert_eq!(calls[1].2["key"], json!("ext"));

        // the usage-only frame at the end is where the counts live
        assert_eq!(r.usage.cache_read, 900);
        assert_eq!(r.usage.input, 100);
        assert_eq!(r.usage.output, 60);
    }

    #[test]
    fn a_stream_without_done_keeps_what_arrived() {
        let sse = concat!(
            r#"data: {"choices":[{"index":0,"delta":{"content":"חצי"}}]}"#,
            "\n",
            r#"data: {"choices":[{"index":0,"delta":{"content":" משפט"}}]}"#,
            "\n",
            r#"data: {"choices":[{"index":0,"delta":{"cont"#, // cut mid-frame
        );
        let (r, streamed, done) = run_stream(sse);
        assert!(!done);
        assert_eq!(streamed, "חצי משפט");
        assert_eq!(r.text(), "חצי משפט");
        assert_eq!(r.stop, StopReason::Other("stream_incomplete".to_string()));
    }

    #[test]
    fn a_mid_stream_error_frame_is_transient_not_bad_request() {
        let seen = std::sync::Mutex::new(String::new());
        let sink = |d: &str| seen.lock().unwrap().push_str(d);
        let mut deltas = DeltaBatch::new(&sink);
        let mut acc = StreamAcc::default();
        let e = acc
            .feed_line(
                r#"data: {"error":{"message":"upstream died","type":"server_error"}}"#,
                &mut deltas,
            )
            .unwrap_err();
        // BadRequest is reserved for the HTTP status — it is the fallback signal.
        assert!(matches!(e, ProviderError::Transient(_)));
        assert!(e.retryable());
    }

    #[test]
    fn stream_tolerates_pings_and_blank_lines() {
        let sse = concat!(
            ": keep-alive\n\n",
            r#"data: {"choices":[{"index":0,"delta":{"content":"א"}}]}"#,
            "\n\n\n",
            r#"data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
            "\n",
            "data: [DONE]\n",
        );
        let (r, streamed, done) = run_stream(sse);
        assert!(done);
        assert_eq!(streamed, "א");
        assert_eq!(r.stop, StopReason::EndTurn);
    }
}
