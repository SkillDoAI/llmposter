# Request Capture

llmposter automatically captures every request it receives. Use the capture API to verify what your client actually sent — not just what it received.

## API

### `server.get_requests() -> Vec<CapturedRequest>`

Returns all captured requests in order.

```rust
let requests = server.get_requests();
assert_eq!(requests.len(), 2);
assert!(requests[0].body.contains("hello"));
```

### `server.request_count() -> usize`

Returns the number of requests captured so far.

```rust
assert_eq!(server.request_count(), 0);
// ... send a request ...
assert_eq!(server.request_count(), 1);
```

### `server.reset()`

Clears all captured requests and resets scenario state.

```rust
server.reset();
assert_eq!(server.request_count(), 0);
```

## CapturedRequest Fields

| Field | Type | Description |
|-------|------|-------------|
| `method` | `String` | HTTP method (always `"POST"` for LLM endpoints) |
| `path` | `String` | Request path (e.g., `"/v1/chat/completions"`) |
| `body` | `String` | Raw request body (JSON string) |
| `matched_scenario` | `Option<String>` | Scenario name if the matched fixture has one |
| `timestamp` | `Instant` | When the request was received |

## Example: Verify Client Sends Correct Requests

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn test_client_sends_correct_model() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("ok"))
        .build()
        .await?;

    // Your client code talks to the mock server
    let client = reqwest::Client::new();
    client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await?;

    // Verify what was sent
    let requests = server.get_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/v1/chat/completions");

    let body: serde_json::Value = serde_json::from_str(&requests[0].body)?;
    assert_eq!(body["model"], "gpt-4");
    assert_eq!(body["messages"][0]["content"], "hello");

    Ok(())
}
```

## Example: Verify Retry Count

```rust
use llmposter::{Fixture, ServerBuilder};
use llmposter::fixture::FailureConfig;

#[tokio::test]
async fn test_client_retries_on_429() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("ok")
                .with_scenario("retry", Some(""), Some("failed"))
                .with_failure(FailureConfig {
                    latency_ms: None,
                    corrupt_body: None,
                    truncate_after_frames: None,
                    disconnect_after_ms: None,
                }),
        )
        .fixture(
            Fixture::new()
                .with_error(429, "Rate limited")
                .with_scenario("retry", Some(""), Some("failed")),
        )
        .fixture(
            Fixture::new()
                .respond_with_content("success after retry")
                .with_scenario("retry", Some("failed"), Some("done")),
        )
        .build()
        .await?;

    // ... client with retry logic sends requests ...

    // After test, verify retry behavior
    let count = server.request_count();
    assert!(count >= 2, "expected at least 1 retry, got {} requests", count);

    Ok(())
}
```

## Notes

- Requests are captured for ALL endpoints (LLM routes, `/code/{N}`, etc.).
- Capture happens before fixture matching — even unmatched (404) requests are captured.
- The `body` field is the raw JSON string, not parsed. Use `serde_json::from_str` to inspect.
- Use `server.reset()` to clear captures between test phases.
