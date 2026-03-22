/// Build a JSON error response body matching the OpenAI error format.
///
/// Real OpenAI errors: {"error": {"message": "...", "type": "...", "param": null, "code": "..."}}
/// Spec: https://platform.openai.com/docs/guides/error-codes
pub fn build_error_body(status: u16, message: &str) -> String {
    let error_type = match status {
        400 => "invalid_request_error",
        401 => "authentication_error",
        403 => "permission_error",
        404 => "not_found_error",
        429 => "rate_limit_error",
        500 | 502 | 503 | 529 => "server_error",
        _ => "api_error",
    };
    let error_code = match status {
        400 => "invalid_request",
        401 => "invalid_api_key",
        403 => "permission_denied",
        404 => "not_found",
        429 => "rate_limit_exceeded",
        500 => "server_error",
        502 => "bad_gateway",
        503 => "service_unavailable",
        529 => "overloaded",
        _ => "error",
    };
    serde_json::json!({
        "error": {
            "message": message,
            "type": error_type,
            "param": null,
            "code": error_code
        }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_build_openai_error_format() {
        let body = build_error_body(429, "Rate limit exceeded");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["error"]["message"], "Rate limit exceeded");
        assert_eq!(json["error"]["type"], "rate_limit_error");
        assert_eq!(json["error"]["code"], "rate_limit_exceeded");
        assert!(json["error"]["param"].is_null(), "param must be null");
    }

    #[test]
    fn should_build_error_body_for_various_status_codes() {
        let cases = [
            (400, "invalid_request_error", "invalid_request"),
            (401, "authentication_error", "invalid_api_key"),
            (429, "rate_limit_error", "rate_limit_exceeded"),
            (500, "server_error", "server_error"),
            (502, "server_error", "bad_gateway"),
            (503, "server_error", "service_unavailable"),
            (529, "server_error", "overloaded"),
        ];
        for (status, expected_type, expected_code) in cases {
            let body = build_error_body(status, "test");
            let json: serde_json::Value = serde_json::from_str(&body).unwrap();
            assert_eq!(json["error"]["type"], expected_type, "status {}", status);
            assert_eq!(json["error"]["code"], expected_code, "status {}", status);
            assert!(json["error"]["param"].is_null());
        }
    }
}
