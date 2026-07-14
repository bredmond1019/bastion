// Exact token counting via vendored `tiktoken-rs` encoders (`cl100k_base` / `o200k_base`).
// Pure module: no I/O, no async, no process/db access.

use std::sync::OnceLock;

use tiktoken_rs::CoreBPE;

/// The tiktoken encoder family a model uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// GPT-4o / GPT-4.1 / o-series models.
    O200kBase,
    /// Everything else (GPT-3.5/4, Claude, embeddings) — the default.
    Cl100kBase,
}

/// Select the tiktoken encoding family for `model`. Unknown model ids fall back
/// to `Cl100kBase` (the default encoder) rather than panicking.
pub fn encoder_for(model: &str) -> Encoding {
    match model {
        "gpt-4o" | "gpt-4o-mini" | "gpt-4.1" | "gpt-4.1-mini" | "gpt-4.1-nano" | "o1"
        | "o1-mini" | "o1-preview" | "o3" | "o3-mini" | "o4-mini" => Encoding::O200kBase,
        _ => Encoding::Cl100kBase,
    }
}

fn cl100k() -> &'static CoreBPE {
    static ENCODER: OnceLock<CoreBPE> = OnceLock::new();
    ENCODER.get_or_init(|| tiktoken_rs::cl100k_base().expect("vendored cl100k_base.tiktoken asset"))
}

fn o200k() -> &'static CoreBPE {
    static ENCODER: OnceLock<CoreBPE> = OnceLock::new();
    ENCODER.get_or_init(|| tiktoken_rs::o200k_base().expect("vendored o200k_base.tiktoken asset"))
}

/// Return the exact tiktoken token count for `text` under the encoder selected by `model`.
/// Empty text always returns `0`. Unknown model ids use the default encoder.
pub fn count(text: &str, model: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    let bpe = match encoder_for(model) {
        Encoding::O200kBase => o200k(),
        Encoding::Cl100kBase => cl100k(),
    };
    bpe.encode_ordinary(text).len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_returns_zero() {
        assert_eq!(count("", "claude-haiku-4-5"), 0);
        assert_eq!(count("", "gpt-4o"), 0);
    }

    #[test]
    fn known_sample_matches_exact_cl100k_count() {
        // "Hello, world!" tokenizes to 4 tokens under cl100k_base: "Hello", ",", " world", "!"
        assert_eq!(count("Hello, world!", "claude-haiku-4-5"), 4);
    }

    #[test]
    fn known_sample_matches_exact_o200k_count() {
        // "Hello, world!" tokenizes to 4 tokens under o200k_base as well.
        assert_eq!(count("Hello, world!", "gpt-4o"), 4);
    }

    #[test]
    fn unknown_model_uses_default_encoder() {
        assert_eq!(
            encoder_for("some-unknown-model-id"),
            Encoding::Cl100kBase,
            "unknown model ids must fall back to the default encoder"
        );
        assert_eq!(
            count("Hello, world!", "some-unknown-model-id"),
            count("Hello, world!", "claude-haiku-4-5"),
            "unknown model falls back to the same default encoder"
        );
    }

    #[test]
    fn o_series_models_select_o200k_base() {
        for model in ["gpt-4o", "gpt-4o-mini", "gpt-4.1", "o1", "o3", "o4-mini"] {
            assert_eq!(encoder_for(model), Encoding::O200kBase, "model: {model}");
        }
    }

    #[test]
    fn claude_and_embedding_models_select_cl100k_base() {
        for model in [
            "claude-opus-4-8",
            "claude-haiku-4-5",
            "text-embedding-3-small",
            "claude-3-5-sonnet-20241022",
        ] {
            assert_eq!(encoder_for(model), Encoding::Cl100kBase, "model: {model}");
        }
    }

    #[test]
    fn longer_text_counts_grow_with_repetition() {
        let one = count(
            "The quick brown fox jumps over the lazy dog.",
            "claude-haiku-4-5",
        );
        let two = count(
            "The quick brown fox jumps over the lazy dog. The quick brown fox jumps over the lazy dog.",
            "claude-haiku-4-5",
        );
        assert_eq!(
            two,
            one * 2,
            "repeating the sentence doubles the token count"
        );
        assert!(one > 0);
    }
}
