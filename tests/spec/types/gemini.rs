//! Golden structs for Gemini generateContent API spec compliance.
//!
//! Spec: https://ai.google.dev/api/generate-content
//! Target: v1beta (latest, 2025)

use serde::Deserialize;

/// GenerateContentResponse — top-level response from Gemini.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpecGenerateContentResponse {
    pub candidates: Vec<SpecCandidate>,
    #[serde(default)]
    pub prompt_feedback: Option<serde_json::Value>,
    #[serde(default)]
    pub usage_metadata: Option<SpecUsageMetadata>,
    #[serde(default)]
    pub model_version: Option<String>,
    #[serde(default)]
    pub response_id: Option<String>,
}

/// A candidate response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpecCandidate {
    pub content: SpecContent,
    #[serde(default)]
    pub index: Option<u64>,
    #[serde(default)]
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub safety_ratings: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    pub citation_metadata: Option<serde_json::Value>,
    #[serde(default)]
    pub avg_logprobs: Option<f64>,
}

/// Content object containing parts.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecContent {
    pub parts: Vec<SpecPart>,
    pub role: String,
}

/// A content part — text or function call.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpecPart {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub function_call: Option<SpecFunctionCall>,
}

/// Function call in a Gemini response.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecFunctionCall {
    pub name: String,
    #[serde(default)]
    pub args: Option<serde_json::Value>,
}

/// Token usage metadata.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpecUsageMetadata {
    #[serde(default)]
    pub prompt_token_count: Option<u64>,
    #[serde(default)]
    pub candidates_token_count: Option<u64>,
    #[serde(default)]
    pub total_token_count: Option<u64>,
    #[serde(default)]
    pub cached_content_token_count: Option<u64>,
}
