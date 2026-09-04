//! AI providers. One trait, three wire formats.

pub mod anthropic;
pub mod gemini;
pub mod openai;

use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use super::types::{ProviderError, ProviderRequest, ProviderResponse};

#[async_trait]
pub trait Provider: Send + Sync {
    async fn complete(
        &self,
        req: &ProviderRequest,
        cancel: &CancellationToken,
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
pub(crate) fn classify_reqwest(e: &reqwest::Error) -> ProviderError {
    ProviderError::Transient(e.to_string())
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
    fn custom_provider_requires_a_base_url() {
        assert!(build("custom", "regular", "k", "").is_err());
        assert!(build("openai", "regular", "", "").is_err());
        assert!(build("claude", "regular", "k", "").is_ok());
        assert!(build("nope", "regular", "k", "").is_err());
    }
}
