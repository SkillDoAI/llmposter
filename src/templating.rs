//! Response templating via [`minijinja`] (gated by the `templating` feature).
//!
//! Fixtures may set `response.content_template` instead of — never alongside
//! — `response.content`. When the handler matches such a fixture it calls
//! [`render`] with a small context of request-derived values. The rendered
//! string is used as the response body exactly where `content` would have
//! been.
//!
//! The template context currently exposes:
//!
//! - `user_message` — the extracted user message string (same value that
//!   fixture matching sees)
//! - `model` — the model name from the request body
//! - `provider` — `"openai"`, `"anthropic"`, `"gemini"`, or `"responses"`
//! - `request` — the full parsed request JSON (use e.g.
//!   `{{ request.messages[-1].content }}` to reach into it)
//!
//! Rendering errors are mapped to a short string and returned to the caller
//! — the handler surfaces them as HTTP 500 rather than crashing the server.
//!
//! ```ignore
//! // fixture.yaml
//! fixtures:
//!   - match:
//!       user_message: "echo"
//!     response:
//!       content_template: "You said: {{ user_message }} (model={{ model }})"
//! ```

use minijinja::{context, Environment};

/// Render `template` using the supplied request context values.
/// Returns `Err(String)` with a human-readable message on failure.
pub(crate) fn render(
    template: &str,
    user_message: &str,
    model: &str,
    provider: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    // A fresh Environment per render is cheap for small templates and keeps
    // the code stateless. For high-throughput servers we could cache compiled
    // templates behind a hash, but a mock server is not the place to optimize.
    let mut env = Environment::new();
    env.add_template("t", template)
        .map_err(|e| format!("template compile error: {}", e))?;
    let tmpl = env
        .get_template("t")
        .map_err(|e| format!("template lookup error: {}", e))?;
    tmpl.render(context! {
        user_message => user_message,
        model => model,
        provider => provider,
        request => request,
    })
    .map_err(|e| format!("template render error: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn should_interpolate_user_message() {
        let out = render(
            "Hello, {{ user_message }}!",
            "world",
            "gpt-4",
            "openai",
            &json!({}),
        )
        .unwrap();
        assert_eq!(out, "Hello, world!");
    }

    #[test]
    fn should_interpolate_model_and_provider() {
        let out = render(
            "{{ provider }}/{{ model }}",
            "",
            "claude-sonnet-4-6",
            "anthropic",
            &json!({}),
        )
        .unwrap();
        assert_eq!(out, "anthropic/claude-sonnet-4-6");
    }

    #[test]
    fn should_reach_into_request_json() {
        let req = json!({
            "messages": [{"role": "user", "content": "deep value"}]
        });
        let out = render(
            "Got: {{ request.messages[0].content }}",
            "ignored",
            "m",
            "openai",
            &req,
        )
        .unwrap();
        assert_eq!(out, "Got: deep value");
    }

    #[test]
    fn should_report_compile_error_for_invalid_syntax() {
        let err = render("Hello {{ unclosed", "w", "m", "p", &json!({})).unwrap_err();
        assert!(
            err.contains("template compile error"),
            "got unexpected error: {}",
            err
        );
    }

    #[test]
    fn should_report_render_error_for_unknown_filter() {
        let err = render(
            "{{ user_message | nonexistent_filter }}",
            "hi",
            "m",
            "p",
            &json!({}),
        )
        .unwrap_err();
        assert!(
            err.contains("template render error"),
            "got unexpected error: {}",
            err
        );
    }

    #[test]
    fn should_render_literal_template_unchanged() {
        let out = render("no placeholders here", "w", "m", "p", &json!({})).unwrap();
        assert_eq!(out, "no placeholders here");
    }
}
