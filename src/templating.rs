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

use minijinja::context;

use crate::fixture::TemplateCache;

/// Render `template` against the supplied request context, using a
/// per-fixture compile cache so hot templated fixtures don't pay the
/// minijinja `add_template` cost on every request.
///
/// Returns `Err(String)` with a human-readable message on compile or
/// render failure.
pub(crate) fn render(
    template: &str,
    cache: &TemplateCache,
    user_message: &str,
    model: &str,
    provider: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    let env = cache.get_or_compile(template).map_err(|e| e.to_string())?;
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

/// Ad-hoc render used by unit tests that don't have a fixture handy.
/// Pays the compile cost on every call; not used on any request hot path.
#[cfg(test)]
fn render_uncached(
    template: &str,
    user_message: &str,
    model: &str,
    provider: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    let mut env = minijinja::Environment::new();
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
        let out = render_uncached(
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
        let out = render_uncached(
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
        let out = render_uncached(
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
        let err = render_uncached("Hello {{ unclosed", "w", "m", "p", &json!({})).unwrap_err();
        assert!(
            err.contains("template compile error"),
            "got unexpected error: {}",
            err
        );
    }

    #[test]
    fn should_report_render_error_for_unknown_filter() {
        let err = render_uncached(
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
        let out = render_uncached("no placeholders here", "w", "m", "p", &json!({})).unwrap();
        assert_eq!(out, "no placeholders here");
    }

    /// Exercises `TemplateCache`'s `Debug` and `Clone` impls so they
    /// aren't untouched by coverage. Clone always returns an empty
    /// cache by design (see the `Clone` impl note in `fixture.rs`).
    #[test]
    fn should_debug_and_clone_template_cache() {
        let cache = TemplateCache::default();
        let before = format!("{:?}", cache);
        assert!(before.contains("TemplateCache"));
        assert!(before.contains("initialized: false"));

        // Populate by rendering once.
        render("ok", &cache, "", "", "", &json!({})).unwrap();
        let after = format!("{:?}", cache);
        assert!(after.contains("initialized: true"));

        // Clone returns a fresh empty cache regardless of source state.
        let cloned = cache.clone();
        let cloned_dbg = format!("{:?}", cloned);
        assert!(cloned_dbg.contains("initialized: false"));
    }

    /// Verify the cache short-circuits compilation after the first call.
    #[test]
    fn should_reuse_compiled_template_from_cache() {
        let cache = TemplateCache::default();
        let first = render(
            "Hello {{ user_message }}",
            &cache,
            "A",
            "m",
            "p",
            &json!({}),
        )
        .unwrap();
        let second = render(
            "Hello {{ user_message }}",
            &cache,
            "B",
            "m",
            "p",
            &json!({}),
        )
        .unwrap();
        assert_eq!(first, "Hello A");
        assert_eq!(second, "Hello B");
    }

    /// Compile errors are cached too — the second call returns the same
    /// error message without re-running the parser.
    #[test]
    fn should_cache_compile_errors_so_second_call_returns_same_message() {
        let cache = TemplateCache::default();
        let first = render("Hello {{ unclosed", &cache, "", "", "", &json!({})).unwrap_err();
        let second = render("Hello {{ unclosed", &cache, "", "", "", &json!({})).unwrap_err();
        assert_eq!(first, second);
        assert!(first.contains("template compile error"));
    }
}
