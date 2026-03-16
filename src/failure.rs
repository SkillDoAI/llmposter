/// Build a JSON error response body for HTTP error fixtures.
pub fn build_error_body(status: u16, message: &str) -> String {
    serde_json::json!({
        "error": {
            "message": message,
            "type": "error",
            "code": status
        }
    })
    .to_string()
}

/// Build a JSON 404 response body when no fixture matches.
pub fn build_no_match_body(model: &str, user_message: &str) -> String {
    serde_json::json!({
        "error": {
            "message": format!(
                "No fixture matched: model='{}', user_message='{}'",
                model, user_message
            ),
            "type": "no_fixture_match"
        }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_build_error_response_body() {
        let body = build_error_body(429, "Rate limit exceeded");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["error"]["message"], "Rate limit exceeded");
        assert_eq!(json["error"]["type"], "error");
        assert_eq!(json["error"]["code"], 429);
    }

    #[test]
    fn should_build_no_match_response() {
        let body = build_no_match_body("gpt-4", "hello world");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        let msg = json["error"]["message"].as_str().unwrap();
        assert!(msg.contains("gpt-4"));
        assert!(msg.contains("hello world"));
        assert_eq!(json["error"]["type"], "no_fixture_match");
    }

    #[test]
    fn should_build_error_body_for_various_status_codes() {
        for status in [400, 429, 500, 502, 503, 529] {
            let body = build_error_body(status, "test");
            let json: serde_json::Value = serde_json::from_str(&body).unwrap();
            assert_eq!(json["error"]["code"], status);
        }
    }
}
