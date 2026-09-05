//! AI providers. One trait, three wire formats.

pub mod anthropic;
pub mod gemini;
pub mod openai;

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::{Stream, StreamExt};
use tokio_util::sync::CancellationToken;

use super::types::{ProviderError, ProviderRequest, ProviderResponse};

/// Where a provider pushes assistant text as it arrives. `Sync` (rather than
/// `Send`) is what makes `&DeltaSink` itself `Send`, keeping the call future
/// `Send` as `async_trait` requires.
pub type DeltaSink<'a> = &'a (dyn Fn(&str) + Sync);

/// A sink that drops everything — used by tests and by call sites that do not
/// have a UI to stream into.
pub fn noop_sink() -> DeltaSink<'static> {
    &|_: &str| {}
}

#[async_trait]
pub trait Provider: Send + Sync {
    /// `on_delta` receives assistant *text* as it streams in. It is called from
    /// the provider read loop, so it must not block.
    async fn complete(
        &self,
        req: &ProviderRequest,
        cancel: &CancellationToken,
        on_delta: DeltaSink<'_>,
    ) -> Result<ProviderResponse, ProviderError>;

    /// The wire family (`claude` / `gemini` / `openai`). Diagnostics only —
    /// the UI shows the user's own provider label, not this.
    #[allow(dead_code)]
    fn id(&self) -> &'static str;
}

// ---------------------------------------------------------------------------
// Shared HTTP client
// ---------------------------------------------------------------------------

static HTTP_AI: OnceLock<reqwest::Client> = OnceLock::new();

/// The one and only client used to talk to AI providers.
///
/// A *flat* request timeout is deliberately absent: an Opus-class turn with a
/// long thinking phase legitimately runs for minutes. The per-turn ceiling is
/// applied by `call_with_deadline` below instead, so it can depend on the model.
pub fn http_ai() -> &'static reqwest::Client {
    HTTP_AI.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(90))
            .user_agent("AI-Yemot-Desktop")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// Per-turn wall-clock ceiling: 10 minutes for the slow/thinking models.
pub fn turn_timeout(model: &str) -> Duration {
    let m = model.to_ascii_lowercase();
    if m.contains("opus") || m.contains("-pro") || m.ends_with("pro") {
        Duration::from_secs(600)
    } else {
        Duration::from_secs(300)
    }
}

/// Run one provider call under both the cancellation token and the per-turn
/// deadline. Neither is a `reqwest` timeout, so both work for streaming too.
pub async fn call_with_deadline<F>(
    model: &str,
    cancel: &CancellationToken,
    fut: F,
) -> Result<ProviderResponse, ProviderError>
where
    F: std::future::Future<Output = Result<ProviderResponse, ProviderError>>,
{
    if cancel.is_cancelled() {
        return Err(ProviderError::Cancelled);
    }
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(ProviderError::Cancelled),
        r = tokio::time::timeout(turn_timeout(model), fut) => match r {
            Ok(inner) => inner,
            Err(_) => Err(ProviderError::Transient("פסק זמן בהמתנה לתשובת המודל".to_string())),
        },
    }
}

// ---------------------------------------------------------------------------
// Streaming plumbing
// ---------------------------------------------------------------------------

/// A stream that produced nothing for this long is treated as dead. It is not
/// the turn ceiling: `call_with_deadline` still bounds the whole read.
pub const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// Coalescing window for text deltas — one Tauri event per ~60ms is smooth to
/// read and cheap; one per token is neither.
const DELTA_INTERVAL: Duration = Duration::from_millis(60);
const DELTA_CHARS: usize = 40;

/// Buffers text deltas and hands them to the sink at most every
/// `DELTA_INTERVAL` or every `DELTA_CHARS` characters, whichever comes first.
pub struct DeltaBatch<'a> {
    sink: DeltaSink<'a>,
    buf: String,
    chars: usize,
    last: Instant,
}

impl<'a> DeltaBatch<'a> {
    pub fn new(sink: DeltaSink<'a>) -> Self {
        DeltaBatch {
            sink,
            buf: String::new(),
            chars: 0,
            last: Instant::now(),
        }
    }

    pub fn push(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.buf.push_str(text);
        self.chars += text.chars().count();
        if self.chars >= DELTA_CHARS || self.last.elapsed() >= DELTA_INTERVAL {
            self.flush();
        }
    }

    /// Emit whatever is buffered. Always called once the stream ends, so no
    /// tail is ever lost.
    pub fn flush(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        (self.sink)(&self.buf);
        self.buf.clear();
        self.chars = 0;
        self.last = Instant::now();
    }
}

impl Drop for DeltaBatch<'_> {
    fn drop(&mut self) {
        self.flush();
    }
}

/// Reads a byte stream line by line. SSE framing is `\n`-delimited, and a
/// chunk boundary can fall anywhere, so the tail is carried across chunks.
pub struct LineReader<S> {
    stream: S,
    buf: Vec<u8>,
    eof: bool,
}

impl<S, B> LineReader<S>
where
    S: Stream<Item = Result<B, reqwest::Error>> + Unpin,
    B: AsRef<[u8]>,
{
    pub fn new(stream: S) -> Self {
        LineReader {
            stream,
            buf: Vec::new(),
            eof: false,
        }
    }

    /// The next line without its terminator, or `None` at end of stream. A
    /// final line with no trailing newline is still returned (a truncated
    /// stream must not silently swallow its last event).
    pub async fn next_line(
        &mut self,
        cancel: &CancellationToken,
    ) -> Result<Option<String>, ProviderError> {
        loop {
            if let Some(pos) = self.buf.iter().position(|b| *b == b'\n') {
                let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                return Ok(Some(String::from_utf8_lossy(&line).into_owned()));
            }
            if self.eof {
                if self.buf.is_empty() {
                    return Ok(None);
                }
                let line = String::from_utf8_lossy(&self.buf).into_owned();
                self.buf.clear();
                return Ok(Some(line));
            }
            let next = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(ProviderError::Cancelled),
                r = tokio::time::timeout(STREAM_IDLE_TIMEOUT, self.stream.next()) => r,
            };
            match next {
                Err(_) => {
                    return Err(ProviderError::Transient(
                        "השידור מספק ה-AI נתקע ללא תשובה".to_string(),
                    ))
                }
                Ok(None) => self.eof = true,
                Ok(Some(Err(e))) => return Err(classify_reqwest(e)),
                Ok(Some(Ok(chunk))) => self.buf.extend_from_slice(chunk.as_ref()),
            }
        }
    }
}

/// The payload of one SSE `data:` line, or `None` for anything else (comments,
/// `event:` lines, blank separators). Providers switch on the JSON's own type
/// field, so the `event:` name is redundant.
pub fn sse_data(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("data:")?;
    let rest = rest.strip_prefix(' ').unwrap_or(rest);
    if rest.is_empty() {
        None
    } else {
        Some(rest)
    }
}

// ---------------------------------------------------------------------------
// Model aliases
// ---------------------------------------------------------------------------

/// `regular` / `pro` / `""` resolve to the provider default; anything else is
/// taken verbatim as an explicit model id typed or picked by the user.
pub fn resolve_model(provider: &str, model: &str) -> String {
    let alias = model.trim().to_ascii_lowercase();
    let is_alias = matches!(alias.as_str(), "regular" | "pro" | "");
    if !is_alias {
        return model.trim().to_string();
    }
    let pro = alias == "pro";
    match provider.to_ascii_lowercase().as_str() {
        "claude" | "anthropic" => {
            if pro {
                "claude-opus-5".to_string()
            } else {
                "claude-sonnet-5".to_string()
            }
        }
        "gemini" => {
            if pro {
                "gemini-2.5-pro".to_string()
            } else {
                "gemini-2.5-flash".to_string()
            }
        }
        "groq" => "llama-3.3-70b-versatile".to_string(),
        // openai + custom (OpenAI-compatible)
        _ => {
            if pro {
                "gpt-4.1".to_string()
            } else {
                "gpt-4.1-mini".to_string()
            }
        }
    }
}

/// Build the provider for a run. `base_url` may be empty for the known ones.
pub fn build(
    provider: &str,
    model: &str,
    api_key: &str,
    base_url: &str,
) -> Result<Box<dyn Provider>, String> {
    let key = api_key.trim().to_string();
    if key.is_empty() {
        return Err("נדרש מפתח API אישי לצורך פנייה ישירה לספק ה-AI".to_string());
    }
    // Gemini puts the model in the URL; the others read it off the request.
    let model = resolve_model(provider, model);
    let base = base_url.trim().trim_end_matches('/').to_string();
    match provider.to_ascii_lowercase().as_str() {
        "claude" | "anthropic" => Ok(Box::new(anthropic::Anthropic::new(key, base))),
        "gemini" => Ok(Box::new(gemini::Gemini::new(key, model, base))),
        "openai" | "groq" | "custom" => {
            let flavour = provider.to_ascii_lowercase();
            if flavour == "custom" && base.is_empty() {
                return Err("נדרשת כתובת API מותאמת אישית עבור ספק מותאם".to_string());
            }
            Ok(Box::new(openai::OpenAiCompatible::new(key, base, &flavour)))
        }
        other => Err(format!("ספק AI '{}' אינו נתמך", other)),
    }
}

// ---------------------------------------------------------------------------
// Shared HTTP error classification
// ---------------------------------------------------------------------------

pub(crate) fn classify_status(status: u16, retry_after: Option<u64>, body: &str) -> ProviderError {
    let detail = short(body);
    match status {
        401 | 403 => ProviderError::Auth(detail),
        429 => ProviderError::RateLimited { retry_after },
        529 => ProviderError::Overloaded,
        400 | 404 | 405 | 413 | 422 => ProviderError::BadRequest(detail),
        s if s >= 500 => ProviderError::Transient(format!("HTTP {} — {}", s, detail)),
        s => ProviderError::BadRequest(format!("HTTP {} — {}", s, detail)),
    }
}

/// Transport failures are always retryable: a timeout, a reset connection and
/// a DNS hiccup are the same class of problem from the loop's point of view.
///
/// The error is taken **by value** so `without_url` can strip the request URL:
/// a provider URL may carry credentials in its query string, and this string
/// ends up in the UI and in logs.
pub(crate) fn classify_reqwest(e: reqwest::Error) -> ProviderError {
    ProviderError::Transient(e.without_url().to_string())
}

/// The provider's own error message, trimmed to something loggable.
pub(crate) fn short(body: &str) -> String {
    let msg = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| e.get("message").or(Some(e)))
                .map(|m| m.as_str().map(str::to_string).unwrap_or_else(|| m.to_string()))
        })
        .unwrap_or_else(|| body.to_string());
    let msg = msg.trim();
    if msg.chars().count() > 300 {
        msg.chars().take(300).collect::<String>() + "…"
    } else {
        msg.to_string()
    }
}

pub(crate) fn retry_after_of(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<f64>().ok())
        .map(|s| s.max(0.0).ceil() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_resolve_per_provider() {
        assert_eq!(resolve_model("claude", "regular"), "claude-sonnet-5");
        assert_eq!(resolve_model("claude", "pro"), "claude-opus-5");
        assert_eq!(resolve_model("gemini", ""), "gemini-2.5-flash");
        assert_eq!(resolve_model("gemini", "pro"), "gemini-2.5-pro");
        assert_eq!(resolve_model("openai", "regular"), "gpt-4.1-mini");
        assert_eq!(resolve_model("openai", "pro"), "gpt-4.1");
        assert_eq!(resolve_model("groq", "pro"), "llama-3.3-70b-versatile");
        // anything else is an explicit id
        assert_eq!(resolve_model("claude", "claude-haiku-4-5"), "claude-haiku-4-5");
        assert_eq!(resolve_model("openai", " gpt-4o "), "gpt-4o");
    }

    #[test]
    fn slow_models_get_the_long_ceiling() {
        assert_eq!(turn_timeout("claude-opus-5").as_secs(), 600);
        assert_eq!(turn_timeout("gemini-2.5-pro").as_secs(), 600);
        assert_eq!(turn_timeout("claude-sonnet-5").as_secs(), 300);
        assert_eq!(turn_timeout("gpt-4.1-mini").as_secs(), 300);
    }

    #[test]
    fn status_classification() {
        assert!(matches!(
            classify_status(401, None, "{}"),
            ProviderError::Auth(_)
        ));
        assert!(matches!(
            classify_status(429, Some(3), "{}"),
            ProviderError::RateLimited { retry_after: Some(3) }
        ));
        assert_eq!(classify_status(529, None, "{}"), ProviderError::Overloaded);
        assert!(classify_status(503, None, "{}").retryable());
        assert!(!classify_status(400, None, "{}").retryable());
    }

    /// A transport error must never quote the request URL: the URL can carry
    /// credentials (a `?key=` query, a signed path) straight into the UI.
    #[tokio::test]
    async fn a_transport_error_never_quotes_the_url() {
        // port 1 on loopback refuses immediately — no network needed.
        let e = http_ai()
            .get("http://127.0.0.1:1/v1beta/models/m:generateContent?key=SECRETKEY")
            .send()
            .await
            .expect_err("a refused connection is expected here");
        let classified = classify_reqwest(e);
        let msg = match &classified {
            ProviderError::Transient(m) => m.clone(),
            other => panic!("expected Transient, got {:?}", other),
        };
        assert!(!msg.contains("SECRETKEY"), "the key leaked: {}", msg);
        assert!(!msg.contains("127.0.0.1"), "the url leaked: {}", msg);
        assert!(!msg.contains("generateContent"), "the url leaked: {}", msg);
        assert!(classified.retryable());
    }

    #[test]
    fn short_extracts_the_provider_message() {
        assert_eq!(
            short(r#"{"error":{"message":"bad key","type":"auth"}}"#),
            "bad key"
        );
    }

    #[test]
    fn provider_ids_are_stable() {
        assert_eq!(build("claude", "regular", "k", "").unwrap().id(), "claude");
        assert_eq!(build("gemini", "regular", "k", "").unwrap().id(), "gemini");
        assert_eq!(build("groq", "regular", "k", "").unwrap().id(), "openai");
    }

    #[test]
    fn sse_data_only_matches_data_lines() {
        assert_eq!(sse_data("data: {\"a\":1}"), Some("{\"a\":1}"));
        // exactly one optional space is stripped, the rest is payload
        assert_eq!(sse_data("data:{\"a\":1}"), Some("{\"a\":1}"));
        assert_eq!(sse_data("data:  x"), Some(" x"));
        assert_eq!(sse_data("data: [DONE]"), Some("[DONE]"));
        assert_eq!(sse_data("event: message_start"), None);
        assert_eq!(sse_data(": keep-alive"), None);
        assert_eq!(sse_data(""), None);
        assert_eq!(sse_data("data:"), None);
    }

    #[test]
    fn deltas_are_batched_by_size_and_flushed_at_the_end() {
        let seen = std::sync::Mutex::new(Vec::<String>::new());
        let sink = |d: &str| seen.lock().unwrap().push(d.to_string());
        {
            let mut b = DeltaBatch::new(&sink);
            // under the char threshold: nothing goes out yet
            b.push("abc");
            assert!(seen.lock().unwrap().is_empty());
            b.push(&"x".repeat(40));
            assert_eq!(seen.lock().unwrap().len(), 1);
            b.push("tail");
        }
        // the drop flushed the tail — no text is ever lost
        let out = seen.lock().unwrap().clone();
        assert_eq!(out.len(), 2);
        assert_eq!(out.concat(), format!("abc{}tail", "x".repeat(40)));
    }

    #[tokio::test]
    async fn line_reader_splits_across_chunk_boundaries() {
        let chunks: Vec<Result<Vec<u8>, reqwest::Error>> = vec![
            Ok(b"data: on".to_vec()),
            Ok("e\r\ndata: t".as_bytes().to_vec()),
            // a Hebrew character split across two chunks must survive
            Ok(vec![0xd7]),
            Ok(vec![0x90, b'\n']),
            Ok(b"data: no-newline-at-eof".to_vec()),
        ];
        let mut r = LineReader::new(futures_util::stream::iter(chunks));
        let cancel = CancellationToken::new();
        let mut lines = Vec::new();
        while let Some(l) = r.next_line(&cancel).await.unwrap() {
            lines.push(l);
        }
        assert_eq!(lines, vec!["data: one", "data: tא", "data: no-newline-at-eof"]);
    }

    #[tokio::test]
    async fn line_reader_honours_cancellation() {
        let (_tx, rx) = tokio::sync::mpsc::channel::<Result<Vec<u8>, reqwest::Error>>(1);
        let mut r = LineReader::new(tokio_stream_of(rx));
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert_eq!(r.next_line(&cancel).await, Err(ProviderError::Cancelled));
    }

    /// A `Stream` over an mpsc receiver, without pulling in `tokio-stream`.
    fn tokio_stream_of(
        rx: tokio::sync::mpsc::Receiver<Result<Vec<u8>, reqwest::Error>>,
    ) -> impl Stream<Item = Result<Vec<u8>, reqwest::Error>> + Unpin {
        Box::pin(futures_util::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|item| (item, rx))
        }))
    }

    #[test]
    fn custom_provider_requires_a_base_url() {
        assert!(build("custom", "regular", "k", "").is_err());
        assert!(build("openai", "regular", "", "").is_err());
        assert!(build("claude", "regular", "k", "").is_ok());
        assert!(build("nope", "regular", "k", "").is_err());
    }
}
