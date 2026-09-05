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
use super::openai::INVALID_ARGUMENTS_KEY;
use super::{
    call_with_deadline, classify_reqwest, classify_status, http_ai, retry_after_of, sse_data,
    DeltaBatch, DeltaSink, LineReader,
};

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
/// Non-streaming ceiling. Streaming has no 10-minute single-response limit, so
/// it can afford twice the room for a long thinking + answer turn.
const MAX_TOKENS: u64 = 16000;
const MAX_TOKENS_STREAM: u64 = 32000;
/// Haiku has no adaptive thinking: it takes an explicit budget instead.
const HAIKU_THINKING_BUDGET: u64 = 4000;

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

/// `thinking: adaptive` and `output_config.effort` are not universal: the 4.5
/// generation (Haiku here) only understands an explicit thinking budget, and
/// rejects `output_config` outright. Everything newer keeps adaptive + effort.
pub fn wants_adaptive_thinking(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    // `adaptive` thinking and `output_config.effort` exist only from the
    // generation after 4-5 / 4-1 (sonnet-4-5, opus-4-1, haiku-4-5): asking an
    // older model for them is a 400.
    !(m.starts_with("claude-haiku-4-5") || m.contains("-4-5") || m.contains("-4-1"))
}

pub fn build_body(req: &ProviderRequest, stream: bool) -> Value {
    let mut body = Map::new();
    body.insert("model".to_string(), json!(req.model));
    body.insert(
        "max_tokens".to_string(),
        json!(if stream { MAX_TOKENS_STREAM } else { MAX_TOKENS }),
    );
    if wants_adaptive_thinking(&req.model) {
        body.insert("thinking".to_string(), json!({ "type": "adaptive" }));
        body.insert("output_config".to_string(), json!({ "effort": "medium" }));
    } else {
        body.insert(
            "thinking".to_string(),
            json!({ "type": "enabled", "budget_tokens": HAIKU_THINKING_BUDGET }),
        );
    }
    body.insert("tools".to_string(), tools_json(&req.tools));
    body.insert("system".to_string(), system_json(&req.system));
    body.insert("messages".to_string(), messages_json(&req.messages));
    if wants_fallback(&req.model) {
        body.insert("fallbacks".to_string(), json!("default"));
    }
    if stream {
        body.insert("stream".to_string(), json!(true));
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
                // An empty text block is noise the runner would echo as a turn.
                "text" => {
                    let t = item.get("text").and_then(|t| t.as_str()).unwrap_or("");
                    if !t.trim().is_empty() {
                        content.push(ContentBlock::Text(t.to_string()));
                    }
                }
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
// SSE accumulator
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum StreamBlock {
    Text(String),
    /// `thinking` + `signature` must both survive to be echoed back verbatim.
    Thinking { thinking: String, signature: String },
    /// `redacted_thinking` (and anything unknown) is kept exactly as it arrived.
    Raw(Value),
    ToolUse {
        id: String,
        name: String,
        json: String,
        /// A `content_block_stop` was seen — the accumulated JSON is complete.
        closed: bool,
    },
}

/// Rebuilds a `ProviderResponse` from the `message_*` / `content_block_*`
/// event stream. Pure: `feed_line` is the only input, so it unit-tests against
/// a fixture string with no network.
#[derive(Default)]
pub struct StreamAcc {
    blocks: Vec<StreamBlock>,
    stop: Option<String>,
    usage: Usage,
}

/// An `error` event mid-stream, mapped onto the same taxonomy as an HTTP status.
fn stream_error(err: &Value) -> ProviderError {
    let kind = err.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let msg = err
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("שגיאה בשידור מ-Anthropic")
        .to_string();
    match kind {
        "overloaded_error" => ProviderError::Overloaded,
        "rate_limit_error" => ProviderError::RateLimited { retry_after: None },
        "authentication_error" | "permission_error" => ProviderError::Auth(msg),
        "invalid_request_error" => ProviderError::BadRequest(msg),
        _ => ProviderError::Transient(msg),
    }
}

impl StreamAcc {
    fn slot(&mut self, index: usize) -> &mut StreamBlock {
        while self.blocks.len() <= index {
            self.blocks.push(StreamBlock::Text(String::new()));
        }
        &mut self.blocks[index]
    }

    fn index_of(ev: &Value) -> usize {
        ev.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize
    }

    /// Feed one raw SSE line. Non-`data:` lines (`event:`, blanks, comments)
    /// are ignored: every Anthropic event carries its own `type`.
    pub fn feed_line(
        &mut self,
        line: &str,
        deltas: &mut DeltaBatch<'_>,
    ) -> Result<(), ProviderError> {
        let Some(data) = sse_data(line) else {
            return Ok(());
        };
        let Ok(ev) = serde_json::from_str::<Value>(data) else {
            // A half-written frame at the end of a truncated stream.
            return Ok(());
        };
        match ev.get("type").and_then(|v| v.as_str()).unwrap_or("") {
            "message_start" => {
                let u = &ev["message"]["usage"];
                let n = |k: &str| u.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
                self.usage.input = n("input_tokens");
                self.usage.cache_read = n("cache_read_input_tokens");
                self.usage.cache_write = n("cache_creation_input_tokens");
                // Some responses report the whole output count here already.
                self.usage.output = self.usage.output.max(n("output_tokens"));
            }
            "content_block_start" => {
                let index = Self::index_of(&ev);
                let b = &ev["content_block"];
                let block = match b.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                    "text" => StreamBlock::Text(
                        b.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    ),
                    "thinking" => StreamBlock::Thinking {
                        thinking: b
                            .get("thinking")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        signature: b
                            .get("signature")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                    },
                    "tool_use" => StreamBlock::ToolUse {
                        id: b.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        name: b.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        json: String::new(),
                        closed: false,
                    },
                    _ => StreamBlock::Raw(b.clone()),
                };
                *self.slot(index) = block;
            }
            "content_block_delta" => {
                let index = Self::index_of(&ev);
                let d = &ev["delta"];
                let text = |k: &str| d.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
                match d.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                    "text_delta" => {
                        let t = text("text");
                        if let StreamBlock::Text(s) = self.slot(index) {
                            s.push_str(&t);
                        }
                        // Thinking is never streamed to the UI — only the answer.
                        deltas.push(&t);
                    }
                    "thinking_delta" => {
                        let t = text("thinking");
                        if let StreamBlock::Thinking { thinking, .. } = self.slot(index) {
                            thinking.push_str(&t);
                        }
                    }
                    "signature_delta" => {
                        let s = text("signature");
                        if let StreamBlock::Thinking { signature, .. } = self.slot(index) {
                            signature.push_str(&s);
                        }
                    }
                    "input_json_delta" => {
                        let p = text("partial_json");
                        if let StreamBlock::ToolUse { json, .. } = self.slot(index) {
                            json.push_str(&p);
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let index = Self::index_of(&ev);
                if let StreamBlock::ToolUse { closed, .. } = self.slot(index) {
                    *closed = true;
                }
            }
            "message_delta" => {
                if let Some(s) = ev["delta"].get("stop_reason").and_then(|v| v.as_str()) {
                    self.stop = Some(s.to_string());
                }
                if let Some(o) = ev["usage"].get("output_tokens").and_then(|v| v.as_u64()) {
                    self.usage.output = o;
                }
                // A `message_delta` may also restate the input side.
                for (k, slot) in [
                    ("input_tokens", &mut self.usage.input),
                    ("cache_read_input_tokens", &mut self.usage.cache_read),
                    ("cache_creation_input_tokens", &mut self.usage.cache_write),
                ] {
                    if let Some(v) = ev["usage"].get(k).and_then(|v| v.as_u64()) {
                        *slot = (*slot).max(v);
                    }
                }
            }
            "error" => return Err(stream_error(&ev["error"])),
            // `ping` / `message_stop` / anything new: nothing to accumulate.
            _ => {}
        }
        Ok(())
    }

    /// Was anything at all received? Decides whether a mid-stream failure is
    /// retryable (nothing yet) or salvageable (partial turn).
    pub fn has_content(&self) -> bool {
        self.blocks.iter().any(|b| match b {
            StreamBlock::Text(t) => !t.trim().is_empty(),
            StreamBlock::Thinking { thinking, .. } => !thinking.is_empty(),
            StreamBlock::Raw(_) => true,
            StreamBlock::ToolUse { .. } => true,
        })
    }

    /// `aborted` = the stream ended early. Tool calls whose JSON never got a
    /// `content_block_stop` are dropped rather than dispatched half-parsed.
    pub fn finish(self, aborted: bool) -> ProviderResponse {
        let mut content = Vec::new();
        for b in self.blocks {
            match b {
                StreamBlock::Text(t) => {
                    if !t.trim().is_empty() {
                        content.push(ContentBlock::Text(t));
                    }
                }
                StreamBlock::Thinking {
                    thinking,
                    signature,
                } => {
                    if !thinking.is_empty() || !signature.is_empty() {
                        content.push(ContentBlock::Thinking(json!({
                            "type": "thinking",
                            "thinking": thinking,
                            "signature": signature,
                        })));
                    }
                }
                StreamBlock::Raw(v) => content.push(ContentBlock::Thinking(v)),
                StreamBlock::ToolUse {
                    id,
                    name,
                    json,
                    closed,
                } => {
                    if aborted && !closed {
                        continue;
                    }
                    let input = if json.trim().is_empty() {
                        json!({})
                    } else {
                        match serde_json::from_str::<Value>(&json) {
                            Ok(v) if v.is_object() => v,
                            // Same shape the OpenAI path uses, so the dispatcher
                            // returns one `is_error` tool result either way.
                            _ => json!({ INVALID_ARGUMENTS_KEY: json }),
                        }
                    };
                    content.push(ContentBlock::ToolUse { id, name, input });
                }
            }
        }
        let stop = if aborted && self.stop.is_none() {
            StopReason::Other("stream_incomplete".to_string())
        } else {
            parse_stop(self.stop.as_deref())
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
impl Provider for Anthropic {
    fn id(&self) -> &'static str {
        "claude"
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

use super::Provider;

impl Anthropic {
    fn request(&self, req: &ProviderRequest, stream: bool) -> reqwest::RequestBuilder {
        let mut rb = http_ai()
            .post(self.endpoint())
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json");
        if wants_fallback(&req.model) {
            rb = rb.header("anthropic-beta", FALLBACK_BETA);
        }
        if stream {
            rb = rb.header("accept", "text/event-stream");
        }
        rb.json(&build_body(req, stream))
    }

    /// Stream first; a 4xx means the endpoint does not speak SSE (custom
    /// gateways), so the whole turn is retried once as a plain request.
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
        loop {
            match lines.next_line(cancel).await {
                Ok(Some(line)) => {
                    if let Err(e) = acc.feed_line(&line, &mut deltas) {
                        deltas.flush();
                        // An error event after real content: keep the partial
                        // turn instead of throwing the tokens away.
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
        let aborted = acc.stop.is_none();
        let had_content = acc.has_content();
        let parsed = acc.finish(aborted);
        if !had_content && aborted {
            // A 200 that yielded no events at all is a gateway that ignored
            // `stream`: fall back to the plain request rather than retrying.
            return Err(ProviderError::BadRequest(
                "השידור מ-Anthropic הסתיים ללא תוכן".to_string(),
            ));
        }
        if parsed.stop == StopReason::Refusal && parsed.tool_uses().is_empty() {
            return Err(ProviderError::Refusal);
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
    use std::sync::Mutex;

    /// Non-streaming body — the shape assertions below are about the request
    /// mapping, which `stream` does not change beyond `max_tokens`.
    fn build_body_t(req: &ProviderRequest) -> Value {
        build_body(req, false)
    }

    /// Runs a fixture through the real line framing and accumulator.
    fn run_stream(sse: &str) -> (ProviderResponse, String) {
        let seen = Mutex::new(String::new());
        let sink = |d: &str| seen.lock().unwrap().push_str(d);
        let mut acc = StreamAcc::default();
        {
            let mut deltas = DeltaBatch::new(&sink);
            for line in sse.split('\n') {
                acc.feed_line(line.trim_end_matches('\r'), &mut deltas).unwrap();
            }
        }
        let aborted = acc.stop.is_none();
        let out = acc.finish(aborted);
        let text = seen.lock().unwrap().clone();
        (out, text)
    }

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
        let b = build_body_t(&fixture_req("claude-sonnet-5"));
        assert_eq!(b["max_tokens"], json!(16000));
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
    fn haiku_gets_an_explicit_budget_and_no_output_config() {
        let b = build_body_t(&fixture_req("claude-haiku-4-5"));
        assert_eq!(b["thinking"]["type"], json!("enabled"));
        assert_eq!(b["thinking"]["budget_tokens"], json!(4000));
        assert!(b.get("output_config").is_none());
        assert!(b["thinking"]["budget_tokens"].as_u64().unwrap() < 16000);

        // dated haiku ids too
        assert!(!wants_adaptive_thinking("claude-haiku-4-5-20251001"));
        // the whole 4-5 / 4-1 generation predates adaptive thinking
        for m in [
            "claude-sonnet-4-5",
            "claude-sonnet-4-5-20250929",
            "claude-opus-4-1",
            "claude-opus-4-1-20250805",
        ] {
            assert!(!wants_adaptive_thinking(m), "{} predates adaptive", m);
            let b = build_body_t(&fixture_req(m));
            assert_eq!(b["thinking"]["type"], json!("enabled"), "{}", m);
            assert!(b.get("output_config").is_none(), "{}", m);
        }
        // everything newer keeps adaptive + effort
        for m in [
            "claude-opus-5",
            "claude-sonnet-5",
            "claude-opus-4-8",
            "claude-sonnet-4-6",
            "claude-fable-5-1",
        ] {
            assert!(wants_adaptive_thinking(m), "{} should stay adaptive", m);
            let b = build_body_t(&fixture_req(m));
            assert_eq!(b["thinking"]["type"], json!("adaptive"), "{}", m);
            assert_eq!(b["output_config"]["effort"], json!("medium"), "{}", m);
        }
    }

    #[test]
    fn empty_text_blocks_are_dropped() {
        let r = parse_response(&json!({
            "content": [
                {"type":"text","text":"   "},
                {"type":"text","text":""},
                {"type":"tool_use","id":"toolu_1","name":"lookup_param","input":{}}
            ],
            "stop_reason": "tool_use",
            "usage": {}
        }));
        assert_eq!(r.content.len(), 1);
        assert_eq!(r.text(), "");
        assert_eq!(r.tool_uses().len(), 1);
    }

    #[test]
    fn opus_opts_into_server_side_fallback() {
        let b = build_body_t(&fixture_req("claude-opus-5"));
        assert_eq!(b["fallbacks"], json!("default"));
        assert!(wants_fallback("claude-opus-5"));
        assert!(!wants_fallback("claude-sonnet-5"));
    }

    #[test]
    fn tools_and_system_are_byte_stable_across_builds() {
        let a = build_body_t(&fixture_req("claude-sonnet-5"));
        let b = build_body_t(&fixture_req("claude-sonnet-5"));
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

    // -----------------------------------------------------------------------
    // Streaming
    // -----------------------------------------------------------------------

    #[test]
    fn streaming_body_doubles_max_tokens_and_sets_stream() {
        let b = build_body(&fixture_req("claude-sonnet-5"), true);
        assert_eq!(b["max_tokens"], json!(32000));
        assert_eq!(b["stream"], json!(true));
        let n = build_body(&fixture_req("claude-sonnet-5"), false);
        assert_eq!(n["max_tokens"], json!(16000));
        assert!(n.get("stream").is_none());
        // everything else is byte-identical, so the cache prefix is unaffected
        assert_eq!(
            serde_json::to_string(&b["tools"]).unwrap(),
            serde_json::to_string(&n["tools"]).unwrap()
        );
        assert_eq!(
            serde_json::to_string(&b["system"]).unwrap(),
            serde_json::to_string(&n["system"]).unwrap()
        );
    }

    const MULTI_BLOCK: &str = concat!(
        "event: message_start\n",
        r#"data: {"type":"message_start","message":{"usage":{"input_tokens":12,"cache_read_input_tokens":900,"cache_creation_input_tokens":7,"output_tokens":1}}}"#,
        "\n\n",
        "event: ping\ndata: {\"type\":\"ping\"}\n\n",
        r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
        "\n",
        r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"אני חושב"}}"#,
        "\n",
        r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":" עוד"}}"#,
        "\n",
        r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"SIG"}}"#,
        "\n",
        r#"data: {"type":"content_block_stop","index":0}"#,
        "\n",
        r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#,
        "\n",
        r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"שלום"}}"#,
        "\n",
        r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":" עולם"}}"#,
        "\n",
        r#"data: {"type":"content_block_stop","index":1}"#,
        "\n",
        r#"data: {"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_7","name":"lookup_param","input":{}}}"#,
        "\n",
        r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"ke"}}"#,
        "\n",
        r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"y\": \"ty"}}"#,
        "\n",
        r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"pe\"}"}}"#,
        "\n",
        r#"data: {"type":"content_block_stop","index":2}"#,
        "\n",
        r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":143}}"#,
        "\n",
        r#"data: {"type":"message_stop"}"#,
        "\n",
    );

    #[test]
    fn stream_rebuilds_thinking_text_and_a_split_tool_call() {
        let (r, streamed) = run_stream(MULTI_BLOCK);
        assert_eq!(r.stop, StopReason::ToolUse);
        // only the answer text is streamed to the UI, never the thinking
        assert_eq!(streamed, "שלום עולם");
        assert_eq!(r.text(), "שלום עולם");

        assert_eq!(r.content.len(), 3);
        match &r.content[0] {
            ContentBlock::Thinking(v) => {
                assert_eq!(v["type"], json!("thinking"));
                assert_eq!(v["thinking"], json!("אני חושב עוד"));
                assert_eq!(v["signature"], json!("SIG"));
            }
            other => panic!("expected thinking, got {:?}", other),
        }
        let calls = r.tool_uses();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "toolu_7");
        assert_eq!(calls[0].1, "lookup_param");
        assert_eq!(calls[0].2["key"], json!("type"));

        assert_eq!(r.usage.input, 12);
        assert_eq!(r.usage.output, 143);
        assert_eq!(r.usage.cache_read, 900);
        assert_eq!(r.usage.cache_write, 7);
    }

    #[test]
    fn a_stream_split_across_chunk_boundaries_parses_the_same() {
        // The reader hands out lines; feeding them one by one must be identical
        // to feeding the whole fixture.
        let (whole, _) = run_stream(MULTI_BLOCK);
        let joined: String = MULTI_BLOCK.split('\n').collect::<Vec<_>>().join("\n");
        let (split, _) = run_stream(&joined);
        assert_eq!(whole.text(), split.text());
        assert_eq!(whole.tool_uses().len(), split.tool_uses().len());
    }

    #[test]
    fn a_truncated_stream_keeps_text_and_drops_the_half_written_tool_call() {
        let sse = concat!(
            r#"data: {"type":"message_start","message":{"usage":{"input_tokens":5}}}"#,
            "\n",
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            "\n",
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"חצי"}}"#,
            "\n",
            r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_9","name":"lookup_param","input":{}}}"#,
            "\n",
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"key\":"}}"#,
            "\n",
            r#"data: {"type":"content_block_del"#, // a frame cut mid-write
        );
        let (r, streamed) = run_stream(sse);
        assert_eq!(streamed, "חצי");
        assert_eq!(r.text(), "חצי");
        // no content_block_stop for the tool -> never dispatched
        assert!(r.tool_uses().is_empty());
        assert_eq!(r.stop, StopReason::Other("stream_incomplete".to_string()));
    }

    #[test]
    fn invalid_tool_json_uses_the_shared_error_shape() {
        let sse = concat!(
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"lookup_param","input":{}}}"#,
            "\n",
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{key: type"}}"#,
            "\n",
            r#"data: {"type":"content_block_stop","index":0}"#,
            "\n",
            r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":4}}"#,
            "\n",
        );
        let (r, _) = run_stream(sse);
        let calls = r.tool_uses();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].2[INVALID_ARGUMENTS_KEY], json!("{key: type"));
    }

    #[test]
    fn an_error_event_maps_onto_the_shared_taxonomy() {
        let seen = Mutex::new(String::new());
        let sink = |d: &str| seen.lock().unwrap().push_str(d);
        let mut deltas = DeltaBatch::new(&sink);
        let mut acc = StreamAcc::default();
        let e = acc
            .feed_line(
                r#"data: {"type":"error","error":{"type":"overloaded_error","message":"busy"}}"#,
                &mut deltas,
            )
            .unwrap_err();
        assert_eq!(e, ProviderError::Overloaded);
        assert!(!acc.has_content());
        assert!(e.retryable());
    }

    #[test]
    fn a_streamed_refusal_keeps_the_non_stream_stop_reason() {
        let sse = concat!(
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            "\n",
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"לא"}}"#,
            "\n",
            r#"data: {"type":"message_delta","delta":{"stop_reason":"refusal"},"usage":{"output_tokens":2}}"#,
            "\n",
        );
        let (r, _) = run_stream(sse);
        assert_eq!(r.stop, StopReason::Refusal);
        assert!(r.tool_uses().is_empty());
    }

    #[test]
    fn redacted_thinking_is_echoed_verbatim() {
        let sse = concat!(
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"redacted_thinking","data":"AAAA"}}"#,
            "\n",
            r#"data: {"type":"content_block_stop","index":0}"#,
            "\n",
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}"#,
            "\n",
        );
        let (r, _) = run_stream(sse);
        match &r.content[0] {
            ContentBlock::Thinking(v) => {
                assert_eq!(v["type"], json!("redacted_thinking"));
                assert_eq!(v["data"], json!("AAAA"));
            }
            other => panic!("expected redacted thinking, got {:?}", other),
        }
        // and it round-trips back into the next request unchanged
        assert_eq!(block_json(&r.content[0])["data"], json!("AAAA"));
    }
}
