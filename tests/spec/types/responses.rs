//! Golden structs for OpenAI Responses API spec compliance.
//!
//! Spec: https://platform.openai.com/docs/api-reference/responses/object
//! Target: latest API version (2025)

use serde::Deserialize;

/// The Responses API response object.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecResponsesResponse {
    pub id: String,
    pub object: String,
    pub status: String,
    pub model: String,
    pub output: Vec<serde_json::Value>,
    pub usage: SpecResponsesUsage,
    // Note: real API returns `metadata` but server doesn't emit it yet.
    // Add when server gains metadata pass-through support.
}

/// Usage statistics for a Responses API request.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecResponsesUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    // TODO: real API includes input_tokens_details and output_tokens_details.
    // Add when server emits them for greater spec fidelity.
    #[serde(default)]
    pub input_tokens_details: Option<serde_json::Value>,
    #[serde(default)]
    pub output_tokens_details: Option<serde_json::Value>,
}

/// A text output item in the response.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecOutputMessage {
    pub id: String,
    #[serde(rename = "type")]
    pub item_type: String,
    pub status: String,
    pub role: String,
    pub content: Vec<SpecOutputContent>,
}

/// Content within a text output item.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecOutputContent {
    #[serde(rename = "type")]
    pub content_type: String,
    // Optional to support non-text content types (e.g., refusal)
    #[serde(default)]
    pub text: Option<String>,
    // Real API returns `annotations` — not yet emitted by server
    #[serde(default)]
    pub annotations: Option<serde_json::Value>,
}

/// A function call output item.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecFunctionCallItem {
    #[serde(rename = "type")]
    pub item_type: String,
    pub id: String,
    pub call_id: String,
    pub status: String,
    pub name: String,
    pub arguments: String,
}
