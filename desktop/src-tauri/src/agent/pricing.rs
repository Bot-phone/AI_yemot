//! BYOK cost meter. Prices are USD per million tokens.
//!
//! An unknown model yields `None` — the UI then omits the cost line rather than
//! showing a number that is quietly wrong.

use super::types::Usage;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Price {
    pub input: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub output: f64,
}

const TABLE: &[(&str, Price)] = &[
    (
        "claude-opus-5",
        Price { input: 5.0, cache_read: 0.5, cache_write: 6.25, output: 25.0 },
    ),
    (
        "claude-sonnet-5",
        Price { input: 2.0, cache_read: 0.2, cache_write: 2.5, output: 10.0 },
    ),
    (
        "claude-haiku-4-5",
        Price { input: 1.0, cache_read: 0.1, cache_write: 1.25, output: 5.0 },
    ),
    (
        "gemini-2.5-flash",
        Price { input: 0.30, cache_read: 0.03, cache_write: 0.0, output: 2.50 },
    ),
    (
        "gemini-2.5-pro",
        Price { input: 1.25, cache_read: 0.125, cache_write: 0.0, output: 10.0 },
    ),
    (
        "gpt-4.1-mini",
        Price { input: 0.4, cache_read: 0.1, cache_write: 0.0, output: 1.6 },
    ),
    (
        "gpt-4.1",
        Price { input: 2.0, cache_read: 0.5, cache_write: 0.0, output: 8.0 },
    ),
    (
        "gpt-4o",
        Price { input: 2.5, cache_read: 1.25, cache_write: 0.0, output: 10.0 },
    ),
];

/// Exact id first, then longest known id that the given model starts with
/// (so `gpt-4.1-mini-2025-04-14` prices as `gpt-4.1-mini`, not as `gpt-4.1`).
pub fn price_for(model: &str) -> Option<Price> {
    let m = model.trim().to_ascii_lowercase();
    if let Some((_, p)) = TABLE.iter().find(|(id, _)| *id == m) {
        return Some(*p);
    }
    TABLE
        .iter()
        .filter(|(id, _)| m.starts_with(id))
        .max_by_key(|(id, _)| id.len())
        .map(|(_, p)| *p)
}

/// Total USD for a run, or `None` when the model is not in the table.
pub fn cost_usd(model: &str, u: &Usage) -> Option<f64> {
    let p = price_for(model)?;
    Some(
        (u.input as f64 * p.input
            + u.cache_read as f64 * p.cache_read
            + u.cache_write as f64 * p.cache_write
            + u.output as f64 * p.output)
            / 1_000_000.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_models_are_priced() {
        let u = Usage { input: 1_000_000, output: 1_000_000, cache_read: 0, cache_write: 0 };
        assert_eq!(cost_usd("claude-opus-5", &u), Some(30.0));
        assert_eq!(cost_usd("claude-sonnet-5", &u), Some(12.0));
        assert_eq!(cost_usd("gpt-4.1-mini", &u), Some(2.0));
    }

    #[test]
    fn cache_read_is_cheap_and_counted_separately() {
        let u = Usage { input: 0, output: 0, cache_read: 1_000_000, cache_write: 1_000_000 };
        assert_eq!(cost_usd("claude-sonnet-5", &u), Some(2.7));
        // Gemini bills no cache write.
        assert_eq!(cost_usd("gemini-2.5-flash", &u), Some(0.03));
    }

    #[test]
    fn dated_ids_fall_back_to_the_longest_prefix() {
        assert_eq!(price_for("gpt-4.1-mini-2025-04-14"), price_for("gpt-4.1-mini"));
        assert_eq!(price_for("claude-sonnet-5-20260219"), price_for("claude-sonnet-5"));
    }

    #[test]
    fn unknown_model_is_none() {
        assert!(cost_usd("llama-3.3-70b-versatile", &Usage::default()).is_none());
        assert!(price_for("mystery-9").is_none());
    }

    #[test]
    fn cache_hit_pct() {
        let u = Usage { input: 100, output: 50, cache_read: 300, cache_write: 0 };
        assert!((u.cache_hit_pct() - 75.0).abs() < 1e-9);
        assert_eq!(Usage::default().cache_hit_pct(), 0.0);
    }
}
