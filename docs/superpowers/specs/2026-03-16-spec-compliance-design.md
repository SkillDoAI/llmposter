# Spec Compliance Test Suite & Metadata Fields

> **Goal:** Make llmposter responses indistinguishable from real LLM API responses. Every field a typed SDK client expects, we emit. TDD from the real API specs.

**Architecture:** Decomposed into 4 sub-projects (one per provider), each shipping as a minor version bump. Spec-faithful "golden" structs live in test code and are derived directly from API docs. Production format modules gain missing metadata fields. Each sub-project follows the same pattern: write spec structs → write failing tests → add fields → pass.

**Tech Stack:** Rust, serde (JSON deserialization validation), reqwest (HTTP assertions), tokio (async tests). No YAML in spec tests — these hit HTTP endpoints and validate JSON responses only.

---

## Versioning Plan

| Version | Provider | Spec Reference |
|---------|----------|----------------|
| 0.3.0 | OpenAI Chat Completions | https://platform.openai.com/docs/api-reference/chat/object |
| 0.3.1 | Anthropic Messages | https://docs.anthropic.com/en/api/messages |
| 0.3.2 | Gemini generateContent | https://ai.google.dev/api/generate-content |
| 0.3.3 | OpenAI Responses API | https://platform.openai.com/docs/api-reference/responses/object |

Each version targets the latest stable API spec. We document the targeted version in source. No backporting of older API versions unless the delta is trivial. A dedicated spec addendum will be written for each version (0.3.1–0.3.3) before its implementation begins.

Every version gets a CHANGELOG.md entry per CLAUDE.md conventions.

---

## Test Structure

```
tests/
  spec/
    mod.rs              # Shared helpers: build server, send request, parse SSE
    openai.rs           # OpenAI Chat Completions spec compliance tests
    anthropic.rs        # Anthropic Messages spec compliance tests
    gemini.rs           # Gemini generateContent spec compliance tests
    responses.rs        # OpenAI Responses API spec compliance tests
    types/
      mod.rs            # Re-exports
      openai.rs         # Golden structs from OpenAI API docs
      anthropic.rs      # Golden structs from Anthropic API docs
      gemini.rs         # Golden structs from Gemini API docs
      responses.rs      # Golden structs from Responses API docs
```

### Server Helpers

`tests/spec/mod.rs` provides shared test utilities following the existing pattern in `tests/openai_test.rs` etc.:

```rust
/// Build a mock server with a text fixture and return (server, client).
pub async fn server_with_text(content: &str) -> (MockServer, reqwest::Client) { ... }

/// Build a mock server with a tool call fixture and return (server, client).
pub async fn server_with_tool_call(name: &str, args: Value) -> (MockServer, reqwest::Client) { ... }

/// Parse SSE stream body into a Vec of (event_type, data_string) pairs.
pub fn parse_sse(body: &str) -> Vec<(Option<String>, String)> { ... }
```

These use `ServerBuilder::new().fixture(...).build().await.unwrap()` on port 0, same as existing integration tests.

### Spec Types (Golden Structs)

Each `types/<provider>.rs` file contains Rust structs derived directly from the provider's API documentation. These are **test-only** — not shipped in the library. They mirror the real API response shape exactly, with every field the spec defines.

**Serde configuration:**
- All golden structs derive `#[derive(Debug, Deserialize)]`
- Golden structs do NOT use `#[serde(deny_unknown_fields)]` — we want deserialization to succeed even if we emit extra fields (forward-compatibility). The shape tests explicitly assert required fields via value checks, not via serde rejection.
- `Option<T>` fields must be verified via explicit assertions (e.g., `assert!(resp.system_fingerprint.is_some())`) because serde silently accepts `null` for `Option`. Shape tests assert that optional fields we claim to emit are actually present.

Each struct gets a doc comment with the canonical spec URL so anyone can verify against the source.

### Test Naming Convention

Spec compliance tests use a `spec_` prefix to distinguish them from behavioral tests (which use `should_` per CLAUDE.md). This is a deliberate deviation — spec tests validate "does this match the API contract?" rather than "does this feature work?".

Format: `spec_{provider}_{what_is_tested}`

### Test Categories

**Shape compliance tests** — one test per response type:
- Non-streaming text response
- Non-streaming tool call response
- Streaming text response
- Streaming tool call response

Each test:
1. Starts a mock server via shared helpers (port 0, ephemeral)
2. Sends a request via reqwest to the mock endpoint
3. Deserializes the response into the spec-faithful golden struct (not our internal types)
4. Asserts required fields are present and non-null (explicit assertions, not just serde success)
5. Asserts field values are correctly typed and have valid values
6. Asserts optional fields we emit are present and correctly shaped

**Semantic compliance tests** — behavioral contracts:
- Correct `finish_reason` / `stop_reason` values per response type
- Streaming event ordering and structure
- Usage token invariants (`total == prompt + completion`)
- Tool call argument serialization format (JSON string vs JSON object)
- Provider-specific protocol requirements (e.g., OpenAI `[DONE]` sentinel, Anthropic `ping` event)

---

## Request Handling Principle

**Accept unknown request fields silently.** Callers may pass `temperature`, `top_p`, `max_tokens`, `tools`, `metadata`, or any other valid request parameter. We extract only what we need (model, messages/input, stream flag) and ignore the rest. We never reject a request because it contains fields we don't use — that would break real client code that sets those parameters.

Only reject requests that are structurally invalid (unparseable JSON, missing required fields like `model` or `messages`).

This is already the current behavior (we parse as `serde_json::Value` and extract specific keys), but it is called out here as a design invariant that spec compliance tests should not violate.

---

## v0.3.0 — OpenAI Chat Completions

### Missing Metadata Fields

Fields to add to `src/format/openai.rs`:

| Field | Type | Value | Location |
|-------|------|-------|----------|
| `system_fingerprint` | `Option<String>` | `"fp_llmposter"` | `ChatCompletionResponse`, `ChatCompletionChunk` |
| `service_tier` | `Option<String>` | `"default"` | `ChatCompletionResponse`, `ChatCompletionChunk` |
| `logprobs` | `Option<Value>` | `null` | `Choice`, `ChunkChoice` |
| `created` | `u64` | unix timestamp | `ChatCompletionChunk` (currently missing from chunks) |
| `refusal` | `Option<String>` | `null` | `Message` (production struct, always null — we don't simulate refusals) |

### Spec-Faithful Golden Structs (OpenAI)

All structs derive `#[derive(Debug, Deserialize)]`. Integer fields use `u64` (the OpenAI spec says `integer` with no bit-width; we use the widest reasonable type for forward-compatibility — production can use `u32` internally since serde coerces).

#### Non-Streaming Response

Derived from https://platform.openai.com/docs/api-reference/chat/object:

```rust
/// OpenAI Chat Completion response object.
/// Spec: https://platform.openai.com/docs/api-reference/chat/object
#[derive(Debug, Deserialize)]
pub struct SpecChatCompletion {
    pub id: String,
    pub object: String,              // "chat.completion"
    pub created: u64,
    pub model: String,
    pub system_fingerprint: Option<String>,
    pub service_tier: Option<String>,
    pub choices: Vec<SpecChoice>,
    pub usage: SpecUsage,
}

#[derive(Debug, Deserialize)]
pub struct SpecChoice {
    pub index: u64,
    pub message: SpecMessage,
    pub finish_reason: String,
    pub logprobs: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct SpecMessage {
    pub role: String,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<SpecToolCall>>,
    pub refusal: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SpecToolCall {
    pub id: String,
    pub index: Option<u64>,          // present in streaming, optional in non-streaming
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: SpecFunction,
}

#[derive(Debug, Deserialize)]
pub struct SpecFunction {
    pub name: String,
    pub arguments: String,           // JSON string, not object
}

#[derive(Debug, Deserialize)]
pub struct SpecUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}
```

#### Streaming Chunk

Derived from https://platform.openai.com/docs/api-reference/chat/streaming:

```rust
/// OpenAI Chat Completion streaming chunk.
/// Spec: https://platform.openai.com/docs/api-reference/chat/streaming
#[derive(Debug, Deserialize)]
pub struct SpecChatCompletionChunk {
    pub id: String,
    pub object: String,              // "chat.completion.chunk"
    pub created: u64,
    pub model: String,
    pub system_fingerprint: Option<String>,
    pub service_tier: Option<String>,
    pub choices: Vec<SpecChunkChoice>,
    // Note: OpenAI supports optional `usage` on final chunk when
    // stream_options.include_usage is set. Deferred to future iteration.
}

#[derive(Debug, Deserialize)]
pub struct SpecChunkChoice {
    pub index: u64,
    pub delta: SpecDelta,
    pub finish_reason: Option<String>,
    pub logprobs: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct SpecDelta {
    pub role: Option<String>,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<SpecToolCallDelta>>,
}

/// Tool call delta in streaming — index is required (identifies which tool call).
#[derive(Debug, Deserialize)]
pub struct SpecToolCallDelta {
    pub index: u64,                  // required in streaming (not optional)
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub call_type: Option<String>,
    pub function: Option<SpecFunctionDelta>,
}

#[derive(Debug, Deserialize)]
pub struct SpecFunctionDelta {
    pub name: Option<String>,
    pub arguments: Option<String>,
}
```

### Shape Tests (OpenAI)

```
spec_openai_non_streaming_text_response_shape
spec_openai_non_streaming_tool_call_response_shape
spec_openai_streaming_text_response_shape
spec_openai_streaming_tool_call_response_shape
```

### Semantic Tests (OpenAI)

```
spec_openai_finish_reason_stop_for_text
spec_openai_finish_reason_tool_calls_for_tools
spec_openai_streaming_first_chunk_has_role
spec_openai_streaming_last_chunk_has_finish_reason
spec_openai_streaming_ends_with_done_sentinel
spec_openai_streaming_chunks_have_created_timestamp
spec_openai_object_field_is_chat_completion
spec_openai_chunk_object_field_is_chat_completion_chunk
spec_openai_usage_total_equals_prompt_plus_completion
spec_openai_tool_call_arguments_are_json_string
spec_openai_system_fingerprint_present
spec_openai_id_format_starts_with_chatcmpl
spec_openai_streaming_tool_call_deltas_have_index
```

---

## Source Documentation

Each format module (`src/format/<provider>.rs`) gets a doc comment header:

```rust
//! OpenAI Chat Completions API format module.
//!
//! Spec: https://platform.openai.com/docs/api-reference/chat/object
//! Streaming: https://platform.openai.com/docs/api-reference/chat/streaming
//! Target: latest API version (2025)
```

Each spec type file (`tests/spec/types/<provider>.rs`) gets matching links.

---

## TDD Workflow

For each sub-project:

1. Write golden structs from the API docs
2. Write tests that deserialize our current output into golden structs
3. Run tests — they fail on missing/mistyped fields
4. Add missing fields to `src/format/<provider>.rs`
5. Update handler code if needed
6. Run tests — they pass
7. Run full suite to check for regressions
8. Update CHANGELOG.md with version entry
9. Commit, push, review cycle

---

## Future Providers (0.3.1–0.3.3)

Same pattern for each. A dedicated spec addendum will be written for each version before its implementation begins. Key metadata gaps per provider (to be fully specced later):

**Anthropic (0.3.1):** `cache_creation_input_tokens`, `cache_read_input_tokens` in usage. Verify `stop_sequence` field. Streaming event completeness.

**Gemini (0.3.2):** `candidate.index`, `promptFeedback`, `safetyRatings[]` on candidates. Verify camelCase throughout.

**Responses API (0.3.3):** `metadata` pass-through, `incomplete_details` on incomplete status. Verify streaming event lifecycle completeness.

---

## User-Facing Documentation

Each provider version ships with a documentation section (in README or a dedicated doc) that covers:

1. **Which API version we target** — e.g., "OpenAI Chat Completions API (latest, 2025)"
2. **Full field list** — every response field we emit, with a brief description
3. **Intentionally omitted fields** — any fields we don't emit, with the reason (e.g., "logprobs: always null — we don't simulate token probabilities"). The goal is 100% field coverage, so this list should be empty or near-empty.
4. **Request fields we accept vs ignore** — clarify that callers can pass any valid request parameter (`temperature`, `top_p`, `max_tokens`, etc.) and we silently ignore them. We only use `model`, `messages`/`input`, and `stream`.
5. **Behavioral differences** — anything where our mock intentionally differs from real behavior (e.g., deterministic IDs, estimated token counts, static `system_fingerprint`).

This documentation is the contract with users. If they're writing test assertions against our output, they need to know exactly what to expect.

---

## Success Criteria

- Every field in the real API response is present in our mock response
- Typed SDK clients (openai-python, anthropic-python, google-generativeai) can deserialize our responses without error
- Spec compliance tests catch any drift when we modify format modules
- Doc comments link to canonical specs for every provider
- CHANGELOG.md updated for each version
