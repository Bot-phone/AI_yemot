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

/// The month this table was last checked against the providers' published
/// price lists.
///
/// The numbers are **public list prices only**. Free tiers, promotional
/// credits, negotiated rates and any provider price change made after this
/// date are invisible to the app, so a reported cost is an estimate of what a
/// BYOK key would be billed, not an invoice. A model that is not in the table
/// yields `None` and no cost (and no date) is reported at all.
pub const PRICE_LIST_DATE: &str = "2026-09";

const TABLE: &[(&str, Price)] = &[
    (
        "claude-fable-5-1",
        Price { input: 10.0, cache_read: 0.25, cache_write: 12.5, output: 50.0 },
    ),
    (
        "claude-opus-5",
        Price { input: 5.0, cache_read: 0.5, cache_write: 6.25, output: 25.0 },
    ),
    (
        "claude-opus-4-8",
        Price { input: 5.0, cache_read: 0.5, cache_write: 6.25, output: 25.0 },
    ),
    (
        "claude-opus-4-7",
        Price { input: 5.0, cache_read: 0.5, cache_write: 6.25, output: 25.0 },
    ),
    (
        "claude-opus-4-6",
        Price { input: 5.0, cache_read: 0.5, cache_write: 6.25, output: 25.0 },
    ),
    (
        "claude-sonnet-5",
        Price { input: 2.0, cache_read: 0.2, cache_write: 2.5, output: 10.0 },
    ),
    (
        "claude-sonnet-4-6",
        Price { input: 3.0, cache_read: 0.3, cache_write: 3.75, output: 15.0 },
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
    // groq — no prompt cache discount, so cache_read is priced as input.
    (
        "llama-3.3-70b-versatile",
        Price { input: 0.59, cache_read: 0.59, cache_write: 0.0, output: 0.79 },
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

/// The price-list month to report next to a cost — `Some` exactly when there
/// is a cost to qualify.
pub fn price_list_date(cost: Option<f64>) -> Option<String> {
    cost.map(|_| PRICE_LIST_DATE.to_string())
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
        assert!(cost_usd("mystery-9", &Usage::default()).is_none());
        assert!(price_for("mystery-9").is_none());
    }

    #[test]
    fn the_newer_claude_models_are_priced() {
        let u = Usage { input: 1_000_000, output: 1_000_000, cache_read: 0, cache_write: 0 };
        assert_eq!(cost_usd("claude-fable-5-1", &u), Some(60.0));
        assert_eq!(cost_usd("claude-opus-4-8", &u), Some(30.0));
        assert_eq!(cost_usd("claude-opus-4-7", &u), Some(30.0));
        assert_eq!(cost_usd("claude-opus-4-6", &u), Some(30.0));
        assert_eq!(cost_usd("claude-sonnet-4-6", &u), Some(18.0));
        assert_eq!(cost_usd("llama-3.3-70b-versatile", &u), Some(1.38));
    }

    #[test]
    fn fable_is_not_matched_by_any_other_prefix() {
        // `claude-fable-5-1` shares no prefix with opus/sonnet — a mismatch here
        // would silently price a $10/M model as a $5/M one.
        assert_eq!(price_for("claude-fable-5-1").unwrap().input, 10.0);
        assert_eq!(price_for("claude-fable-5-1-20260401").unwrap().input, 10.0);
        assert_ne!(price_for("claude-opus-4-8"), price_for("claude-fable-5-1"));
        // and the longest prefix still wins inside the claude family
        assert_eq!(price_for("claude-sonnet-4-6-20260101"), price_for("claude-sonnet-4-6"));
        assert_ne!(price_for("claude-sonnet-4-6"), price_for("claude-sonnet-5"));
    }

    #[test]
    fn price_list_date_is_present_exactly_when_a_cost_is() {
        let u = Usage::default();
        assert_eq!(
            price_list_date(cost_usd("claude-opus-5", &u)),
            Some(PRICE_LIST_DATE.to_string())
        );
        // a zero cost is still a cost — the date qualifies it
        assert_eq!(cost_usd("claude-opus-5", &u), Some(0.0));
        assert_eq!(price_list_date(cost_usd("mystery-9", &u)), None);
        assert_eq!(price_list_date(None), None);
    }

    #[test]
    fn cache_hit_pct() {
        let u = Usage { input: 100, output: 50, cache_read: 300, cache_write: 0 };
        assert!((u.cache_hit_pct() - 75.0).abs() < 1e-9);
        assert_eq!(Usage::default().cache_hit_pct(), 0.0);
    }
}
