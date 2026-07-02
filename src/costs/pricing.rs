// Hardcoded per-model price table for USD estimation.
// All prices are per million tokens (MTok).
// Unknown models → $0.00 (unpriced; surfaced in CostSummary).

/// Per-model price in USD per million tokens.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelPrice {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
}

/// Look up the price for `model`. Returns `None` for unknown models.
/// Comparison is case-sensitive and exact (model IDs are stable strings).
pub fn price_for(model: &str) -> Option<ModelPrice> {
    let (input, output) = match model {
        // Current Claude models (as of 2026-06)
        "claude-opus-4-8" => (5.00, 25.00),
        "claude-opus-4-7" => (5.00, 25.00),
        "claude-opus-4-6" => (5.00, 25.00),
        "claude-sonnet-4-6" => (3.00, 15.00),
        "claude-haiku-4-5" => (1.00, 5.00),

        // Retired Claude 3.x / 3.5 models (used in fixtures; retired but still
        // appear in older events rows)
        "claude-3-5-haiku-20241022" => (0.80, 4.00),
        "claude-3-5-sonnet-20241022" => (3.00, 15.00),
        "claude-3-5-sonnet-20240620" => (3.00, 15.00),
        "claude-3-opus-20240229" => (15.00, 75.00),
        "claude-3-haiku-20240307" => (0.25, 1.25),
        "claude-3-sonnet-20240229" => (3.00, 15.00),

        // OpenAI embeddings (present in fixtures)
        "text-embedding-3-small" => (0.02, 0.00),
        "text-embedding-3-large" => (0.13, 0.00),
        "text-embedding-ada-002" => (0.10, 0.00),

        _ => return None,
    };
    Some(ModelPrice {
        input_per_mtok: input,
        output_per_mtok: output,
    })
}

/// Estimate cost in USD for the given model and token counts.
/// Returns `0.0` for unknown models.
pub fn estimate_usd(model: &str, tokens_in: u64, tokens_out: u64) -> f64 {
    match price_for(model) {
        Some(p) => {
            tokens_in as f64 / 1_000_000.0 * p.input_per_mtok
                + tokens_out as f64 / 1_000_000.0 * p.output_per_mtok
        }
        None => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_model_returns_exact_usd() {
        // claude-3-5-haiku: $0.80 in / $4.00 out per MTok
        // 2048 in + 256 out
        // = 2048/1e6 * 0.80 + 256/1e6 * 4.00
        // = 0.0016384 + 0.001024 = 0.0026624
        let usd = estimate_usd("claude-3-5-haiku-20241022", 2048, 256);
        assert!(
            (usd - 0.0026624).abs() < 1e-9,
            "expected 0.0026624, got {usd}"
        );
    }

    #[test]
    fn embedding_model_only_charges_input() {
        // text-embedding-3-small: $0.02 in / $0.00 out per MTok
        // output_per_mtok == 0 → output tokens contribute nothing
        let usd_with_output = estimate_usd("text-embedding-3-small", 512, 1_000_000);
        let usd_no_output = estimate_usd("text-embedding-3-small", 512, 0);
        assert_eq!(
            usd_with_output, usd_no_output,
            "embedding model must not charge for output tokens"
        );
        // 512/1e6 * 0.02 = 0.00001024
        assert!(
            (usd_no_output - 0.00001024).abs() < 1e-12,
            "expected 0.00001024, got {usd_no_output}"
        );
    }

    #[test]
    fn unknown_model_returns_zero() {
        let usd = estimate_usd("gpt-4o-mini", 1_000_000, 1_000_000);
        assert_eq!(usd, 0.0, "unknown model must return 0.0");
    }

    #[test]
    fn zero_tokens_returns_zero() {
        let usd = estimate_usd("claude-opus-4-8", 0, 0);
        assert_eq!(usd, 0.0, "zero tokens must produce $0.00");
    }

    #[test]
    fn price_for_known_model_returns_some() {
        let p = price_for("claude-opus-4-8").unwrap();
        assert_eq!(p.input_per_mtok, 5.00);
        assert_eq!(p.output_per_mtok, 25.00);
    }

    #[test]
    fn price_for_unknown_model_returns_none() {
        assert!(price_for("completely-unknown-model-xyz").is_none());
    }

    #[test]
    fn current_claude_models_have_prices() {
        for model in &["claude-opus-4-8", "claude-sonnet-4-6", "claude-haiku-4-5"] {
            assert!(
                price_for(model).is_some(),
                "current model '{model}' must be in the pricing table"
            );
        }
    }
}
