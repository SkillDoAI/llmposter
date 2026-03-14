pub mod anthropic;
pub mod gemini;
pub mod openai;
pub mod responses;

use std::sync::atomic::{AtomicU64, Ordering};

/// Global ID counter for deterministic response IDs.
/// Each server instance gets its own counter, enabling snapshot testing.
pub struct IdGenerator {
    counter: AtomicU64,
}

impl IdGenerator {
    pub fn new() -> Self {
        Self {
            counter: AtomicU64::new(1),
        }
    }

    pub fn next_openai(&self) -> String {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("chatcmpl-llmposter-{}", n)
    }

    pub fn next_anthropic(&self) -> String {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("msg-llmposter-{}", n)
    }

    pub fn next_responses(&self) -> String {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("resp-llmposter-{}", n)
    }
}

impl Default for IdGenerator {
    fn default() -> Self {
        Self::new()
    }
}

/// Estimate token count from text (rough: chars / 4, rounded up).
pub fn estimate_tokens(text: &str) -> u64 {
    (text.len() as u64).div_ceil(4)
}

/// Provider identifier — which endpoint was hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    OpenAI,
    Anthropic,
    Gemini,
    Responses,
}

impl Provider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::OpenAI => "openai",
            Provider::Anthropic => "anthropic",
            Provider::Gemini => "gemini",
            Provider::Responses => "responses",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_generate_sequential_openai_ids() {
        let gen = IdGenerator::new();
        assert_eq!(gen.next_openai(), "chatcmpl-llmposter-1");
        assert_eq!(gen.next_openai(), "chatcmpl-llmposter-2");
    }

    #[test]
    fn should_generate_sequential_anthropic_ids() {
        let gen = IdGenerator::new();
        assert_eq!(gen.next_anthropic(), "msg-llmposter-1");
        assert_eq!(gen.next_anthropic(), "msg-llmposter-2");
    }

    #[test]
    fn should_generate_sequential_responses_ids() {
        let gen = IdGenerator::new();
        assert_eq!(gen.next_responses(), "resp-llmposter-1");
        assert_eq!(gen.next_responses(), "resp-llmposter-2");
    }

    #[test]
    fn should_share_counter_across_providers() {
        let gen = IdGenerator::new();
        assert_eq!(gen.next_openai(), "chatcmpl-llmposter-1");
        assert_eq!(gen.next_anthropic(), "msg-llmposter-2");
        assert_eq!(gen.next_responses(), "resp-llmposter-3");
    }

    #[test]
    fn should_estimate_tokens() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("hi"), 1);
        assert_eq!(estimate_tokens("hello world"), 3);
    }

    #[test]
    fn should_return_provider_str() {
        assert_eq!(Provider::OpenAI.as_str(), "openai");
        assert_eq!(Provider::Anthropic.as_str(), "anthropic");
        assert_eq!(Provider::Gemini.as_str(), "gemini");
        assert_eq!(Provider::Responses.as_str(), "responses");
    }
}
