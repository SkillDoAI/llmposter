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
    // Forward-compat fields from real Responses API spec.
    // Not yet emitted by server but accepted here so deny_unknown_fields
    // won't block future spec compliance improvements.
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    #[serde(default)]
    pub created_at: Option<u64>,
    #[serde(default)]
    pub expires_at: Option<u64>,
    #[serde(default)]
    pub error: Option<serde_json::Value>,
    #[serde(default)]
    pub incomplete_details: Option<serde_json::Value>,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub max_output_tokens: Option<u64>,
    #[serde(default)]
    pub parallel_tool_calls: Option<bool>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub tool_choice: Option<serde_json::Value>,
    #[serde(default)]
    pub top_p: Option<f64>,
    #[serde(default)]
    pub truncation: Option<serde_json::Value>,
}

/// Usage statistics for a Responses API request.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecResponsesUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
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
