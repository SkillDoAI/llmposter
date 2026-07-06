//! Response-side redaction: scrub sensitive substrings from recorded
//! content and tool-call arguments before they hit the cassette.

use super::cassette::RecordedFixture;

// consumed by Task 3/4 (recorder wiring)
#[allow(dead_code)]
pub(crate) const REDACTED: &str = "[REDACTED]";

fn redact_str(s: &str, redactions: &[regex::Regex]) -> String {
    let mut out = s.to_string();
    for re in redactions {
        // NoExpand: the replacement is a literal, not a $-expansion template.
        out = re.replace_all(&out, regex::NoExpand(REDACTED)).into_owned();
    }
    out
}

fn redact_value(v: &mut serde_json::Value, redactions: &[regex::Regex]) {
    match v {
        serde_json::Value::String(s) => *s = redact_str(s, redactions),
        serde_json::Value::Array(items) => {
            items.iter_mut().for_each(|i| redact_value(i, redactions))
        }
        serde_json::Value::Object(map) => {
            map.values_mut().for_each(|i| redact_value(i, redactions))
        }
        _ => {}
    }
}

/// Redact response-side text only. The match key is left alone on purpose —
/// masking it would break replay matching (documented in docs/recording.md).
// consumed by Task 3/4 (recorder wiring)
#[allow(dead_code)]
pub(crate) fn apply_redactions(rec: &mut RecordedFixture, redactions: &[regex::Regex]) {
    if redactions.is_empty() {
        return;
    }
    if let Some(content) = rec.response.content.as_mut() {
        *content = redact_str(content, redactions);
    }
    if let Some(calls) = rec.response.tool_calls.as_mut() {
        for call in calls {
            redact_value(&mut call.arguments, redactions);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{RecordedMatch, RecordedResponse, RecordedToolCall, RECORDED_PRIORITY};

    #[test]
    fn should_redact_content_and_tool_arguments() {
        let redactions = vec![regex::Regex::new(r"sk-[A-Za-z0-9]+").unwrap()];
        let mut rec = RecordedFixture {
            match_rule: RecordedMatch {
                user_message: "q".to_string(),
                model: "gpt-test".to_string(),
            },
            provider: "openai",
            priority: RECORDED_PRIORITY,
            response: RecordedResponse {
                content: Some("key is sk-abc123 ok".to_string()),
                tool_calls: Some(vec![RecordedToolCall {
                    name: "f".to_string(),
                    arguments: serde_json::json!({"token": "sk-zzz9", "nested": {"v": "sk-qqq1"}}),
                }]),
                ..Default::default()
            },
        };
        apply_redactions(&mut rec, &redactions);
        assert_eq!(
            rec.response.content.as_deref(),
            Some("key is [REDACTED] ok")
        );
        let args = &rec.response.tool_calls.as_ref().unwrap()[0].arguments;
        assert_eq!(args["token"], "[REDACTED]");
        assert_eq!(args["nested"]["v"], "[REDACTED]");
    }
}
