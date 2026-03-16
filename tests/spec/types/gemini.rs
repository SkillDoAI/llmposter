//! Golden structs for Gemini generateContent API spec compliance.
//!
//! Spec: https://ai.google.dev/api/generate-content
//! Target: v1beta (latest, 2025)

use serde::Deserialize;

/// GenerateContentResponse — top-level response from Gemini.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpecGenerateContentResponse {
    // May be absent when safety-blocked (only promptFeedback returned)
    #[serde(default)]
    pub candidates: Vec<SpecCandidate>,
    #[serde(default)]
    pub prompt_feedback: Option<serde_json::Value>,
    #[serde(default)]
    pub usage_metadata: Option<SpecUsageMetadata>,
    #[serde(default)]
    pub model_version: Option<String>,
    #[serde(default)]
    pub response_id: Option<String>,
    #[serde(default)]
    pub model_status: Option<serde_json::Value>,
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
    pub finish_message: Option<String>,
    #[serde(default)]
    pub safety_ratings: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    pub citation_metadata: Option<serde_json::Value>,
    #[serde(default)]
    pub avg_logprobs: Option<f64>,
    #[serde(default)]
    pub token_count: Option<u64>,
    #[serde(default)]
    pub grounding_attributions: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    pub grounding_metadata: Option<serde_json::Value>,
    #[serde(default)]
    pub logprobs_result: Option<serde_json::Value>,
    #[serde(default)]
    pub url_context_metadata: Option<serde_json::Value>,
}

/// Content object containing parts.
/// Note: rename_all = "camelCase" is currently a no-op (parts/role are lowercase),
/// but applied for consistency and to catch future snake_case field additions.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpecContent {
    pub parts: Vec<SpecPart>,
    // Role is optional per Gemini spec (may be omitted in some responses)
    #[serde(default)]
    pub role: Option<String>,
}

/// A content part — text or function call.
/// Note: Gemini Part is a oneof with 8+ variants. We model text and functionCall
/// (what the mock server emits) plus forward-compat fields (thought,
/// executableCode, codeExecutionResult, thoughtSignature). deny_unknown_fields
/// will catch if the server adds new variants without updating this struct.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpecPart {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub function_call: Option<SpecFunctionCall>,
    // Forward-compat: thinking/code-execution variants
    #[serde(default)]
    pub thought: Option<bool>,
    #[serde(default)]
    pub thought_signature: Option<serde_json::Value>,
    #[serde(default)]
    pub executable_code: Option<serde_json::Value>,
    #[serde(default)]
    pub code_execution_result: Option<serde_json::Value>,
}

/// Function call in a Gemini response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpecFunctionCall {
    pub name: String,
    /// Args is always emitted by server — non-optional to match FunctionCallPart.
    pub args: serde_json::Value,
    // Gemini 2.x adds `id` for multi-turn function calling — accepted here
    // for forward-compat so deny_unknown_fields won't break if server emits it.
    #[serde(default)]
    pub args: Option<serde_json::Value>,
    // Gemini 2.x adds `id` for multi-turn function calling — accepted here
    // for forward-compat so deny_unknown_fields won't break if server emits it.
    #[serde(default)]
    pub id: Option<String>,
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
    // Forward-compat: thinking/tool-use token counts
    #[serde(default)]
    pub tool_use_prompt_token_count: Option<u64>,
    #[serde(default)]
    pub thoughts_token_count: Option<u64>,
    #[serde(default)]
    pub prompt_tokens_details: Option<serde_json::Value>,
    #[serde(default)]
    pub cache_tokens_details: Option<serde_json::Value>,
    #[serde(default)]
    pub candidates_tokens_details: Option<serde_json::Value>,
    #[serde(default)]
    pub tool_use_prompt_tokens_details: Option<serde_json::Value>,
}
