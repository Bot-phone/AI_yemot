//! Provider-neutral conversation model.
//!
//! Every provider maps *into* these types on the way out and *out of* them on
//! the way back, so the runner never sees a provider-specific shape.

use serde::Serialize;
use serde_json::Value;

// ---------------------------------------------------------------------------
// Conversation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

/// One block of message content.
#[derive(Debug, Clone)]
pub enum ContentBlock {
    Text(String),
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        /// Tool name — needed by Gemini, which keys `functionResponse` by name.
        name: String,
        content: String,
        is_error: bool,
    },
    /// Anthropic thinking / redacted_thinking, echoed back verbatim.
    Thinking(Value),
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

impl Message {
    pub fn user_text(text: impl Into<String>) -> Message {
        Message {
            role: Role::User,
            content: vec![ContentBlock::Text(text.into())],
        }
    }
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    ReadOnly,
    Mutating,
}

#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
    pub kind: ToolKind,
}

// ---------------------------------------------------------------------------
// Responses
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Refusal,
    Other(String),
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Usage {
    pub fn add(&mut self, other: &Usage) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
    }

    /// `cache_read / (input + cache_read + cache_write)` as a percentage.
    pub fn cache_hit_pct(&self) -> f64 {
        let total = self.input + self.cache_read + self.cache_write;
        if total == 0 {
            0.0
        } else {
            (self.cache_read as f64 * 100.0) / total as f64
        }
    }
}

/// What the runner sends to a provider for one turn.
#[derive(Debug, Clone)]
pub struct ProviderRequest {
    /// Resolved, explicit model id (never `regular` / `pro`).
    pub model: String,
    /// System blocks. Anthropic keeps them separate (the last one carries the
    /// cache breakpoint); the others join them with a blank line.
    pub system: Vec<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    /// 1-based turn number — Gemini mints tool-call ids from it.
    pub turn: u32,
}

#[derive(Debug, Clone)]
pub struct ProviderResponse {
    pub content: Vec<ContentBlock>,
    pub stop: StopReason,
    pub usage: Usage,
}

impl ProviderResponse {
    pub fn tool_uses(&self) -> Vec<(&str, &str, &Value)> {
        self.content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::ToolUse { id, name, input } => {
                    Some((id.as_str(), name.as_str(), input))
                }
                _ => None,
            })
            .collect()
    }

    pub fn text(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        for b in &self.content {
            if let ContentBlock::Text(t) = b {
                if !t.trim().is_empty() {
                    parts.push(t.as_str());
                }
            }
        }
        parts.join("\n\n")
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    /// HTTP 429. `retry_after` is the header value in seconds, when present.
    RateLimited { retry_after: Option<u64> },
    /// HTTP 529 / "overloaded".
    Overloaded,
    /// 5xx, connection reset, timeout — retryable.
    Transient(String),
    /// 401 / 403 — a bad API key. Never retried.
    Auth(String),
    /// 400 / 422 — a malformed request. Never retried.
    BadRequest(String),
    /// `stop_reason: "refusal"` — the model declined.
    Refusal,
    /// The run was cancelled between / during the call.
    Cancelled,
}

impl ProviderError {
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            ProviderError::RateLimited { .. }
                | ProviderError::Overloaded
                | ProviderError::Transient(_)
        )
    }

    /// Maps to the `code` field of `agent:error`.
    pub fn code(&self) -> &'static str {
        match self {
            ProviderError::Auth(_) => "auth",
            ProviderError::BadRequest(_) => "bad_request",
            ProviderError::Transient(_) => "network",
            ProviderError::RateLimited { .. } | ProviderError::Overloaded => "provider",
            ProviderError::Refusal => "provider",
            ProviderError::Cancelled => "internal",
        }
    }

    pub fn message(&self) -> String {
        match self {
            ProviderError::RateLimited { retry_after } => match retry_after {
                Some(s) => format!("חריגה ממגבלת הקצב אצל ספק ה-AI (נסה שוב בעוד {} שנ׳)", s),
                None => "חריגה ממגבלת הקצב אצל ספק ה-AI".to_string(),
            },
            ProviderError::Overloaded => "ספק ה-AI עמוס כרגע".to_string(),
            ProviderError::Transient(m) => format!("שגיאת תקשורת מול ספק ה-AI: {}", m),
            ProviderError::Auth(m) => format!("מפתח ה-API אינו תקין: {}", m),
            ProviderError::BadRequest(m) => format!("בקשה שגויה לספק ה-AI: {}", m),
            ProviderError::Refusal => "המודל סירב לבצע את הבקשה".to_string(),
            ProviderError::Cancelled => "הריצה בוטלה".to_string(),
        }
    }
}
