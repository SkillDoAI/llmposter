//! Reassemble a completed SSE stream body into a [`RecordedFixture`].
//!
//! Streaming record mode tees the upstream bytes to the client while
//! buffering them; once the stream ends cleanly the buffered body lands
//! here. Each provider has its own frame grammar, but the recording
//! rules are shared: the provider's completion sentinel is REQUIRED
//! (a truncated stream never records), tool calls win over text, and
//! anything unextractable yields `None`.

use std::collections::BTreeMap;

use crate::format::Provider;

use super::extract::{accumulate_gemini_parts, finish, string_args_or_warn};
use super::{extract_responses, OpenAiEndpoint, RecordedFixture, RecordedToolCall};

/// Split an SSE body into (event, data) pairs. `event` is empty for
/// event-less streams (OpenAI, Gemini). `[DONE]` is returned verbatim.
///
/// Frames split on `\n\n`; `event:`/`data:` line prefixes with the
/// optional single leading space stripped per the SSE spec;
/// multi-data-line frames joined with `\n`.
fn parse_sse(body: &str) -> Vec<(String, String)> {
    // Normalize CRLF/CR line endings to LF up front — real providers
    // delimit frames with \r\n\r\n. Safe: the SSE grammar forbids raw CR
    // or LF inside a field value (they only ever terminate lines), so
    // this rewrite can never alter data bytes.
    let body = body.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = Vec::new();
    for frame in body.split("\n\n") {
        let mut event = String::new();
        let mut data_lines: Vec<&str> = Vec::new();
        for line in frame.lines() {
            if let Some(rest) = line.strip_prefix("event:") {
                event = strip_one_space(rest).to_string();
            } else if let Some(rest) = line.strip_prefix("data:") {
                data_lines.push(strip_one_space(rest));
            }
        }
        if !event.is_empty() || !data_lines.is_empty() {
            out.push((event, data_lines.join("\n")));
        }
    }
    out
}

/// Strip AT MOST one leading space — per the SSE spec, a single space
/// after the field colon is cosmetic; any further spaces are data.
fn strip_one_space(s: &str) -> &str {
    s.strip_prefix(' ').unwrap_or(s)
}

/// Dispatch to the per-provider reassembler. `endpoint` discriminates
/// the OpenAI endpoints sharing `Provider::OpenAI`; embeddings never
/// streams, so its arm is always `None`.
pub(crate) fn reassemble_for(
    provider: Provider,
    endpoint: OpenAiEndpoint,
    body: &str,
    model: &str,
    user_message: &str,
) -> Option<RecordedFixture> {
    match provider {
        Provider::OpenAI => match endpoint {
            OpenAiEndpoint::Chat => reassemble_openai(body, model, user_message),
            OpenAiEndpoint::Completions => reassemble_completions(body, model, user_message),
            OpenAiEndpoint::Embeddings => None, // embeddings never streams
        },
        Provider::Anthropic => reassemble_anthropic(body, model, user_message),
        Provider::Gemini => reassemble_gemini(body, model, user_message),
        Provider::Responses => reassemble_responses(body, model, user_message),
    }
}

/// `true` when the stream ended with the OpenAI-family `data: [DONE]`
/// sentinel — chat and legacy completions share it.
fn ends_with_done(frames: &[(String, String)]) -> bool {
    frames.last().map(|(_, d)| d.as_str()) == Some("[DONE]")
}

/// OpenAI Chat Completions stream: concatenate `delta.content`
/// fragments; accumulate `delta.tool_calls` fragments keyed by `index`
/// (name arrives once, argument string fragments concatenate); take the
/// last non-null `finish_reason`.
fn reassemble_openai(body: &str, model: &str, user_message: &str) -> Option<RecordedFixture> {
    let frames = parse_sse(body);
    if !ends_with_done(&frames) {
        return None;
    }
    let mut content = String::new();
    let mut finish_reason: Option<String> = None;
    // index → (name once it arrives, concatenated argument fragments)
    let mut tools: BTreeMap<u64, (Option<String>, String)> = BTreeMap::new();
    for (_, data) in &frames {
        if data == "[DONE]" {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };
        let Some(choice) = v.get("choices").and_then(|c| c.get(0)) else {
            continue;
        };
        if let Some(delta) = choice.get("delta") {
            if let Some(t) = delta.get("content").and_then(|c| c.as_str()) {
                content.push_str(t);
            }
            if let Some(calls) = delta.get("tool_calls").and_then(|c| c.as_array()) {
                for (pos, call) in calls.iter().enumerate() {
                    let index = call
                        .get("index")
                        .and_then(|i| i.as_u64())
                        .unwrap_or(pos as u64);
                    let entry = tools.entry(index).or_default();
                    let function = call.get("function");
                    if let Some(name) = function
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str())
                    {
                        entry.0.get_or_insert_with(|| name.to_string());
                    }
                    if let Some(frag) = function
                        .and_then(|f| f.get("arguments"))
                        .and_then(|a| a.as_str())
                    {
                        entry.1.push_str(frag);
                    }
                }
            }
        }
        if let Some(r) = choice.get("finish_reason").and_then(|r| r.as_str()) {
            finish_reason = Some(r.to_string());
        }
    }
    finish(
        Provider::OpenAI,
        model,
        user_message,
        Some(content).filter(|c| !c.is_empty()),
        assemble_fragmented_tools(tools),
        None,
        finish_reason,
    )
}

/// Index-accumulated OpenAI tool fragments → tool calls. Fragments that
/// never received a name are dropped; an empty argument accumulation
/// (zero-arg call) records `{}` silently.
fn assemble_fragmented_tools(
    tools: BTreeMap<u64, (Option<String>, String)>,
) -> Vec<RecordedToolCall> {
    let mut out = Vec::new();
    for (name, args) in tools.into_values() {
        let Some(name) = name else {
            continue;
        };
        let arguments = if args.is_empty() {
            serde_json::json!({})
        } else {
            string_args_or_warn(&name, Some(&serde_json::Value::String(args)))
        };
        out.push(RecordedToolCall { name, arguments });
    }
    out
}

/// Legacy text completions stream: concatenate `choices[0].text`
/// verbatim; take the last non-null `finish_reason`.
fn reassemble_completions(body: &str, model: &str, user_message: &str) -> Option<RecordedFixture> {
    let frames = parse_sse(body);
    if !ends_with_done(&frames) {
        return None;
    }
    let mut content = String::new();
    let mut finish_reason: Option<String> = None;
    for (_, data) in &frames {
        if data == "[DONE]" {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };
        let Some(choice) = v.get("choices").and_then(|c| c.get(0)) else {
            continue;
        };
        if let Some(t) = choice.get("text").and_then(|t| t.as_str()) {
            content.push_str(t);
        }
        if let Some(r) = choice.get("finish_reason").and_then(|r| r.as_str()) {
            finish_reason = Some(r.to_string());
        }
    }
    finish(
        Provider::OpenAI,
        model,
        user_message,
        Some(content).filter(|c| !c.is_empty()),
        Vec::new(),
        None,
        finish_reason,
    )
}

/// A content block being accumulated from an Anthropic stream, tracked
/// by index from its `content_block_start`. Non-replayable block types
/// (`thinking`, ...) are never inserted.
enum Block {
    Text(String),
    Tool { name: String, partial_json: String },
}

/// Anthropic Messages stream: blocks are opened by `content_block_start`
/// (text vs tool_use), grown by `content_block_delta` (`text_delta` /
/// `input_json_delta`), and the `message_stop` event is the required
/// completion sentinel. Frame identity comes from the data `type` field —
/// every Anthropic data payload carries it, so the `event:` line is
/// redundant.
fn reassemble_anthropic(body: &str, model: &str, user_message: &str) -> Option<RecordedFixture> {
    let mut saw_message_stop = false;
    let mut blocks: BTreeMap<u64, Block> = BTreeMap::new();
    let mut stop_reason: Option<String> = None;
    for (_, data) in &parse_sse(body) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };
        match v.get("type").and_then(|t| t.as_str()) {
            Some("content_block_start") => {
                let Some(index) = v.get("index").and_then(|i| i.as_u64()) else {
                    continue;
                };
                let Some(cb) = v.get("content_block") else {
                    continue;
                };
                match cb.get("type").and_then(|t| t.as_str()) {
                    Some("text") => {
                        let initial = cb.get("text").and_then(|t| t.as_str()).unwrap_or("");
                        blocks.insert(index, Block::Text(initial.to_string()));
                    }
                    Some("tool_use") => {
                        if let Some(name) = cb.get("name").and_then(|n| n.as_str()) {
                            blocks.insert(
                                index,
                                Block::Tool {
                                    name: name.to_string(),
                                    partial_json: String::new(),
                                },
                            );
                        }
                    }
                    _ => {} // thinking / redacted_thinking / unknown — skip
                }
            }
            Some("content_block_delta") => {
                let Some(index) = v.get("index").and_then(|i| i.as_u64()) else {
                    continue;
                };
                let Some(delta) = v.get("delta") else {
                    continue;
                };
                match (
                    blocks.get_mut(&index),
                    delta.get("type").and_then(|t| t.as_str()),
                ) {
                    (Some(Block::Text(text)), Some("text_delta")) => {
                        if let Some(t) = delta.get("text").and_then(|t| t.as_str()) {
                            text.push_str(t);
                        }
                    }
                    (Some(Block::Tool { partial_json, .. }), Some("input_json_delta")) => {
                        if let Some(frag) = delta.get("partial_json").and_then(|p| p.as_str()) {
                            partial_json.push_str(frag);
                        }
                    }
                    _ => {} // delta for a skipped block, or unknown delta type
                }
            }
            Some("message_delta") => {
                if let Some(r) = v
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(|r| r.as_str())
                {
                    stop_reason = Some(r.to_string());
                }
            }
            Some("message_stop") => saw_message_stop = true,
            _ => {} // ping / message_start / content_block_stop — nothing to take
        }
    }
    if !saw_message_stop {
        return None;
    }
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for block in blocks.into_values() {
        match block {
            Block::Text(t) => text.push_str(&t),
            Block::Tool { name, partial_json } => {
                let arguments = if partial_json.is_empty() {
                    serde_json::json!({})
                } else {
                    string_args_or_warn(&name, Some(&serde_json::Value::String(partial_json)))
                };
                tool_calls.push(RecordedToolCall { name, arguments });
            }
        }
    }
    finish(
        Provider::Anthropic,
        model,
        user_message,
        Some(text).filter(|t| !t.is_empty()),
        tool_calls,
        stop_reason,
        None,
    )
}

/// Gemini SSE stream: each data frame is a full generateContent chunk —
/// `text` parts concatenate across frames, `functionCall` parts become
/// tool calls. The completion sentinel is a `finishReason` on the LAST
/// candidate-bearing frame; that frame's value is what gets recorded.
fn reassemble_gemini(body: &str, model: &str, user_message: &str) -> Option<RecordedFixture> {
    let mut text = String::new();
    let mut tool_calls: Vec<RecordedToolCall> = Vec::new();
    let mut finish_reason: Option<String> = None;
    let mut last_had_finish = false;
    let mut saw_candidate = false;
    for (_, data) in &parse_sse(body) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };
        let Some(candidate) = v.get("candidates").and_then(|c| c.get(0)) else {
            continue;
        };
        saw_candidate = true;
        last_had_finish = candidate.get("finishReason").is_some();
        // Reassigned every candidate frame, so the recorded value is the
        // LAST frame's — the one carrying the completion sentinel.
        finish_reason = candidate
            .get("finishReason")
            .and_then(|r| r.as_str())
            .map(str::to_string);
        let Some(parts) = candidate
            .get("content")
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.as_array())
        else {
            continue;
        };
        accumulate_gemini_parts(parts, &mut text, &mut tool_calls);
    }
    if !saw_candidate || !last_had_finish {
        return None; // truncated — the final chunk never arrived
    }
    finish(
        Provider::Gemini,
        model,
        user_message,
        Some(text).filter(|t| !t.is_empty()),
        tool_calls,
        None,
        finish_reason,
    )
}

/// Responses API stream: the `response.completed` event carries the
/// entire final response object, so reassembly is just finding it and
/// handing its `response` field to the non-streaming extractor. The
/// event name is matched from the `event:` line OR the data `type`
/// field — both real OpenAI and llmposter send both.
fn reassemble_responses(body: &str, model: &str, user_message: &str) -> Option<RecordedFixture> {
    for (event, data) in parse_sse(body).iter().rev() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };
        if event != "response.completed"
            && v.get("type").and_then(|t| t.as_str()) != Some("response.completed")
        {
            continue;
        }
        return extract_responses(v.get("response")?, model, user_message);
    }
    None // no completed event — truncated, never record
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Canned OpenAI text stream — shared by the LF test and the CRLF
    /// regression test so both parse byte-identical frame content.
    const OPENAI_TEXT_STREAM: &str = concat!(
        "data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1728933352,\"model\":\"gpt-4o-2024-08-06\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1728933352,\"model\":\"gpt-4o-2024-08-06\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hel\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1728933352,\"model\":\"gpt-4o-2024-08-06\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1728933352,\"model\":\"gpt-4o-2024-08-06\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );

    /// Canned Anthropic text stream — shared by the LF test and the CRLF
    /// regression test.
    const ANTHROPIC_TEXT_STREAM: &str = concat!(
        "event: ping\ndata: {\"type\":\"ping\"}\n\n",
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4-6\",\"content\":[],\"stop_reason\":null,\"usage\":{\"input_tokens\":10,\"output_tokens\":0}}}\n\n",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi \"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"there\"}}\n\n",
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":12}}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    );

    #[test]
    fn should_join_multi_data_lines_and_parse_event_prefixes() {
        let frames = parse_sse("event: foo\ndata: line1\ndata: line2\n\ndata: [DONE]\n\n");
        assert_eq!(
            frames,
            vec![
                ("foo".to_string(), "line1\nline2".to_string()),
                (String::new(), "[DONE]".to_string()),
            ]
        );
    }

    #[test]
    fn should_reassemble_openai_stream() {
        let rec = reassemble_for(
            Provider::OpenAI,
            OpenAiEndpoint::Chat,
            OPENAI_TEXT_STREAM,
            "gpt-4o",
            "say hello",
        )
        .unwrap();
        assert_eq!(rec.provider, "openai");
        assert_eq!(rec.match_rule.model, "gpt-4o");
        assert_eq!(rec.match_rule.user_message, "say hello");
        assert_eq!(rec.response.content.as_deref(), Some("Hello"));
        assert_eq!(rec.response.finish_reason.as_deref(), Some("stop"));
        assert!(rec.response.tool_calls.is_none());
    }

    #[test]
    fn should_reassemble_openai_fragmented_tool_call_arguments() {
        let body = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_abc123\",\"type\":\"function\",\"function\":{\"name\":\"get_weather\",\"arguments\":\"\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"city\\\":\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"SF\\\"}\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let rec = reassemble_for(
            Provider::OpenAI,
            OpenAiEndpoint::Chat,
            body,
            "gpt-4o",
            "weather?",
        )
        .unwrap();
        let calls = rec.response.tool_calls.as_ref().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(
            calls[0].arguments["city"], "SF",
            "fragmented argument string reassembled then parsed"
        );
        assert!(rec.response.content.is_none());
        assert_eq!(rec.response.finish_reason.as_deref(), Some("tool_calls"));
    }

    #[test]
    fn should_reassemble_openai_stream_with_crlf_delimiters() {
        // Real providers delimit frames with \r\n\r\n — the CRLF variant
        // must reassemble identically to the LF variant.
        let body = OPENAI_TEXT_STREAM.replace('\n', "\r\n");
        let rec = reassemble_for(
            Provider::OpenAI,
            OpenAiEndpoint::Chat,
            &body,
            "gpt-4o",
            "say hello",
        )
        .unwrap();
        assert_eq!(rec.response.content.as_deref(), Some("Hello"));
        assert_eq!(rec.response.finish_reason.as_deref(), Some("stop"));
        assert!(rec.response.tool_calls.is_none());
    }

    #[test]
    fn should_reassemble_anthropic_stream_with_crlf_delimiters() {
        let body = ANTHROPIC_TEXT_STREAM.replace('\n', "\r\n");
        let rec = reassemble_for(
            Provider::Anthropic,
            OpenAiEndpoint::Chat,
            &body,
            "claude-sonnet-4-6",
            "greet me",
        )
        .unwrap();
        assert_eq!(rec.response.content.as_deref(), Some("Hi there"));
        assert_eq!(rec.response.stop_reason.as_deref(), Some("end_turn"));
        assert!(rec.response.tool_calls.is_none());
    }

    #[test]
    fn should_parse_prefixes_without_trailing_space() {
        // `data:`/`event:` with no space after the colon are spec-legal;
        // at most ONE leading space is stripped — further spaces are data.
        let frames = parse_sse("event:e\ndata:one\ndata:  two\n\ndata:[DONE]\n\n");
        assert_eq!(
            frames,
            vec![
                ("e".to_string(), "one\n two".to_string()),
                (String::new(), "[DONE]".to_string()),
            ]
        );
    }

    #[test]
    fn should_reject_openai_stream_without_done_sentinel() {
        let body =
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hel\"},\"finish_reason\":null}]}\n\n";
        assert!(
            reassemble_for(Provider::OpenAI, OpenAiEndpoint::Chat, body, "m", "u").is_none(),
            "truncated streams never record"
        );
    }

    #[test]
    fn should_reassemble_anthropic_stream() {
        let rec = reassemble_for(
            Provider::Anthropic,
            OpenAiEndpoint::Chat,
            ANTHROPIC_TEXT_STREAM,
            "claude-sonnet-4-6",
            "greet me",
        )
        .unwrap();
        assert_eq!(rec.provider, "anthropic");
        assert_eq!(rec.response.content.as_deref(), Some("Hi there"));
        assert_eq!(rec.response.stop_reason.as_deref(), Some("end_turn"));
        assert!(rec.response.tool_calls.is_none());
    }

    #[test]
    fn should_reassemble_anthropic_tool_stream_via_partial_json() {
        let body = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_2\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"stop_reason\":null}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"lookup\",\"input\":{}}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"q\\\":\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"x\\\"}\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":9}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        );
        let rec = reassemble_for(
            Provider::Anthropic,
            OpenAiEndpoint::Chat,
            body,
            "claude-sonnet-4-6",
            "look up x",
        )
        .unwrap();
        let calls = rec.response.tool_calls.as_ref().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "lookup");
        assert_eq!(
            calls[0].arguments["q"], "x",
            "partial_json fragments reassembled then parsed"
        );
        assert_eq!(rec.response.stop_reason.as_deref(), Some("tool_use"));
    }

    #[test]
    fn should_reject_anthropic_stream_without_message_stop() {
        let body = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_3\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"stop_reason\":null}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"partial\"}}\n\n",
        );
        assert!(
            reassemble_for(
                Provider::Anthropic,
                OpenAiEndpoint::Chat,
                body,
                "claude-sonnet-4-6",
                "u"
            )
            .is_none(),
            "no message_stop means truncated — never record"
        );
    }

    #[test]
    fn should_reassemble_gemini_sse_stream() {
        let body = concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"ans\"}],\"role\":\"model\"},\"index\":0}]}\n\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"wer\"}],\"role\":\"model\"},\"finishReason\":\"STOP\",\"index\":0}],\"usageMetadata\":{\"promptTokenCount\":4,\"candidatesTokenCount\":5,\"totalTokenCount\":9}}\n\n",
        );
        let rec = reassemble_for(
            Provider::Gemini,
            OpenAiEndpoint::Chat,
            body,
            "gemini-2.5-flash",
            "ask",
        )
        .unwrap();
        assert_eq!(rec.provider, "gemini");
        assert_eq!(rec.response.content.as_deref(), Some("answer"));
        assert!(rec.response.tool_calls.is_none());
        assert_eq!(
            rec.response.finish_reason.as_deref(),
            Some("STOP"),
            "finishReason from the LAST frame (the sentinel) is recorded"
        );

        // Last frame WITHOUT finishReason → truncated → None.
        let truncated = concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"ans\"}],\"role\":\"model\"},\"index\":0}]}\n\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"wer\"}],\"role\":\"model\"},\"index\":0}]}\n\n",
        );
        assert!(reassemble_for(
            Provider::Gemini,
            OpenAiEndpoint::Chat,
            truncated,
            "gemini-2.5-flash",
            "ask"
        )
        .is_none());
    }

    #[test]
    fn should_reassemble_gemini_sse_tool_call() {
        let body = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"get_weather\",\"args\":{\"city\":\"SF\"}}}],\"role\":\"model\"},\"finishReason\":\"STOP\",\"index\":0}]}\n\n";
        let rec = reassemble_for(
            Provider::Gemini,
            OpenAiEndpoint::Chat,
            body,
            "gemini-2.5-flash",
            "weather?",
        )
        .unwrap();
        let calls = rec.response.tool_calls.as_ref().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(calls[0].arguments["city"], "SF");
        assert!(rec.response.content.is_none());
        assert_eq!(rec.response.finish_reason.as_deref(), Some("STOP"));
    }

    #[test]
    fn should_reassemble_responses_stream_from_completed_event() {
        let body = concat!(
            "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\",\"object\":\"response\",\"status\":\"in_progress\",\"output\":[]},\"sequence_number\":0}\n\n",
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"item_id\":\"msg_1\",\"output_index\":0,\"delta\":\"IGNORED\",\"sequence_number\":1}\n\n",
            "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"object\":\"response\",\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"id\":\"msg_1\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"resp text\",\"annotations\":[]}]}]},\"sequence_number\":2}\n\n",
        );
        let rec = reassemble_for(
            Provider::Responses,
            OpenAiEndpoint::Chat,
            body,
            "gpt-4o",
            "respond",
        )
        .unwrap();
        assert_eq!(rec.provider, "responses");
        assert_eq!(
            rec.response.content.as_deref(),
            Some("resp text"),
            "content comes from the completed event only — deltas are ignored"
        );

        // No response.completed event → None.
        let no_completed = concat!(
            "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_2\",\"object\":\"response\",\"status\":\"in_progress\",\"output\":[]},\"sequence_number\":0}\n\n",
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"item_id\":\"msg_2\",\"output_index\":0,\"delta\":\"partial\",\"sequence_number\":1}\n\n",
        );
        assert!(reassemble_for(
            Provider::Responses,
            OpenAiEndpoint::Chat,
            no_completed,
            "gpt-4o",
            "respond"
        )
        .is_none());
    }

    #[test]
    fn should_return_none_for_embeddings_endpoint_reassembly() {
        // Embeddings never streams — the dispatch arm is a hard None.
        assert!(reassemble_for(
            Provider::OpenAI,
            OpenAiEndpoint::Embeddings,
            "data: [DONE]\n\n",
            "m",
            "u"
        )
        .is_none());
    }

    #[test]
    fn should_skip_unparseable_and_choiceless_openai_frames() {
        let body = concat!(
            "data: not-json\n\n",
            "data: {\"object\":\"no.choices.here\"}\n\n",
            // Choice with no delta at all — skipped, finish_reason still read.
            "data: {\"choices\":[{\"index\":0,\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\n",
            "data: [DONE]\n\n",
        );
        let rec = reassemble_for(Provider::OpenAI, OpenAiEndpoint::Chat, body, "m", "u").unwrap();
        assert_eq!(rec.response.content.as_deref(), Some("ok"));
    }

    #[test]
    fn should_drop_nameless_tool_fragments_and_record_zero_arg_calls() {
        let body = concat!(
            // index 0: argument fragments only, never a name — dropped.
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{}\"}}]},\"finish_reason\":null}]}\n\n",
            // index 1: name only, zero argument fragments — records {}.
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":1,\"function\":{\"name\":\"zero_arg\"}}]},\"finish_reason\":null}]}\n\n",
            // index 2: entry with no function field at all — contributes nothing.
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":2}]},\"finish_reason\":null}]}\n\n",
            "data: [DONE]\n\n",
        );
        let rec = reassemble_for(Provider::OpenAI, OpenAiEndpoint::Chat, body, "m", "u").unwrap();
        let calls = rec.response.tool_calls.as_ref().unwrap();
        assert_eq!(calls.len(), 1, "nameless fragments dropped: {:?}", calls);
        assert_eq!(calls[0].name, "zero_arg");
        assert_eq!(calls[0].arguments, serde_json::json!({}));
    }

    #[test]
    fn should_reject_completions_stream_without_done_sentinel() {
        let body = "data: {\"choices\":[{\"text\":\"partial\",\"index\":0}]}\n\n";
        assert!(
            reassemble_for(
                Provider::OpenAI,
                OpenAiEndpoint::Completions,
                body,
                "m",
                "u"
            )
            .is_none(),
            "truncated completions streams never record"
        );
    }

    #[test]
    fn should_skip_unparseable_and_choiceless_completions_frames() {
        let body = concat!(
            "data: garbage\n\n",
            "data: {\"object\":\"text_completion\"}\n\n",
            "data: {\"choices\":[{\"text\":\"kept\",\"index\":0,\"finish_reason\":null}]}\n\n",
            "data: [DONE]\n\n",
        );
        let rec = reassemble_for(
            Provider::OpenAI,
            OpenAiEndpoint::Completions,
            body,
            "m",
            "u",
        )
        .unwrap();
        assert_eq!(rec.response.content.as_deref(), Some("kept"));
    }

    #[test]
    fn should_skip_malformed_anthropic_frames_and_thinking_blocks() {
        let body = concat!(
            // Unparseable data frame.
            "data: not-json\n\n",
            // content_block_start without index.
            "data: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"text\",\"text\":\"lost\"}}\n\n",
            // content_block_start without content_block.
            "data: {\"type\":\"content_block_start\",\"index\":9}\n\n",
            // Non-replayable thinking block — never inserted.
            "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"hmm\"}}\n\n",
            // Delta without index.
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"lost\"}}\n\n",
            // Delta without a delta payload.
            "data: {\"type\":\"content_block_delta\",\"index\":0}\n\n",
            // Delta targeting the skipped thinking block.
            "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"hmm\"}}\n\n",
            // The one real text block.
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"kept\"}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );
        let rec =
            reassemble_for(Provider::Anthropic, OpenAiEndpoint::Chat, body, "m", "u").unwrap();
        assert_eq!(
            rec.response.content.as_deref(),
            Some("kept"),
            "only the well-formed text block survives"
        );
        assert!(rec.response.tool_calls.is_none());
    }

    #[test]
    fn should_record_zero_arg_anthropic_tool_call() {
        // tool_use block that never receives an input_json_delta —
        // records with empty {} arguments.
        let body = concat!(
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"t1\",\"name\":\"ping\",\"input\":{}}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );
        let rec =
            reassemble_for(Provider::Anthropic, OpenAiEndpoint::Chat, body, "m", "u").unwrap();
        let calls = rec.response.tool_calls.as_ref().unwrap();
        assert_eq!(calls[0].name, "ping");
        assert_eq!(calls[0].arguments, serde_json::json!({}));
    }

    #[test]
    fn should_skip_malformed_gemini_frames_and_default_missing_args() {
        let body = concat!(
            // Unparseable data frame.
            "data: nonsense\n\n",
            // Frame without candidates.
            "data: {\"usageMetadata\":{\"totalTokenCount\":1}}\n\n",
            // Candidate without content/parts.
            "data: {\"candidates\":[{\"index\":0}]}\n\n",
            // functionCall without a name — dropped.
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"args\":{\"a\":1}}}],\"role\":\"model\"},\"index\":0}]}\n\n",
            // functionCall with no args — records {}.
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"noargs\"}}],\"role\":\"model\"},\"finishReason\":\"STOP\",\"index\":0}]}\n\n",
        );
        let rec = reassemble_for(Provider::Gemini, OpenAiEndpoint::Chat, body, "m", "u").unwrap();
        let calls = rec.response.tool_calls.as_ref().unwrap();
        assert_eq!(calls[0].name, "noargs");
        assert_eq!(calls[0].arguments, serde_json::json!({}));
    }

    #[test]
    fn should_skip_unparseable_responses_frames() {
        // rev() iteration hits the LAST frame first: the unparseable
        // completed frame is skipped, then the valid one records.
        let body = concat!(
            "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"object\":\"response\",\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"id\":\"m1\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"ok\",\"annotations\":[]}]}]},\"sequence_number\":0}\n\n",
            "event: response.completed\ndata: not-json\n\n",
        );
        let rec =
            reassemble_for(Provider::Responses, OpenAiEndpoint::Chat, body, "m", "u").unwrap();
        assert_eq!(rec.response.content.as_deref(), Some("ok"));
    }

    #[test]
    fn should_reassemble_completions_stream() {
        let body = concat!(
            "data: {\"id\":\"cmpl-1\",\"object\":\"text_completion\",\"created\":1,\"model\":\"davinci-002\",\"choices\":[{\"text\":\" legacy\",\"index\":0,\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"cmpl-1\",\"object\":\"text_completion\",\"created\":1,\"model\":\"davinci-002\",\"choices\":[{\"text\":\" completion\",\"index\":0,\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"cmpl-1\",\"object\":\"text_completion\",\"created\":1,\"model\":\"davinci-002\",\"choices\":[{\"text\":\"\",\"index\":0,\"finish_reason\":\"length\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let rec = reassemble_for(
            Provider::OpenAI,
            OpenAiEndpoint::Completions,
            body,
            "davinci-002",
            "legacy prompt",
        )
        .unwrap();
        assert_eq!(rec.provider, "openai");
        assert_eq!(
            rec.response.content.as_deref(),
            Some(" legacy completion"),
            "text fragments concatenated verbatim"
        );
        assert_eq!(rec.response.finish_reason.as_deref(), Some("length"));
    }
}
