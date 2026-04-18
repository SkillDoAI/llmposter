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
| `method` | `String` | HTTP method (`"POST"` for LLM endpoints, `"GET"` for `/code/{status}`) |
| `path` | `String` | Request path (e.g., `"/v1/chat/completions"`; Gemini captures the real `/v1beta/models/{model}:{action}` path as of v0.4.5) |
| `body` | `String` | Raw request body (JSON string). Empty for `GET /code/{status}` and auth-rejected requests. |
| `outcome` | `RequestOutcome` | How the server handled the request — see below (v0.4.5+) |
| `matched_scenario` | `Option<String>` | Scenario name if a scenario-matched fixture served the request |
| `timestamp` | `Instant` | When the request was received |

`CapturedRequest` is marked `#[non_exhaustive]` so future fields can
land without a semver break — use the field accessors instead of
exhaustive destructuring.

### `RequestOutcome` (v0.4.5+)

| Variant | When it's set |
|---------|---------------|
| `Matched` | A fixture was selected, **regardless of its final HTTP status**. Covers `response:` 200s, `error:` fixtures (e.g. `.with_error(429, …)`), `refusal:` fixtures, and all streaming chaos paths (`corrupt_body`, `truncate_after_frames`, `disconnect_after_ms`). The fixture *was* chosen; what it did on the wire afterward doesn't change the capture classification. |
| `NoFixtureMatch` | Request reached an LLM endpoint but no fixture matched (HTTP 404). |
| `BadRequest` | Malformed JSON, failed request extraction, or an invalid `/code/{status}` path (HTTP 400). |
| `AuthRejected` | Rejected by the bearer-token middleware (HTTP 401). |
| `CodeEndpoint` | Hit the `/code/{status}` echo endpoint with a valid status code. |

Use `captured.was_matched()` as a shorthand for `outcome == Matched`.
Remember that `Matched` means "a fixture was selected", NOT "got HTTP
200" — it includes `error:` fixtures returning 4xx/5xx and refusals:

```rust
let reqs = server.get_requests();
let matched: Vec<_> = reqs.iter().filter(|r| r.was_matched()).collect();
assert_eq!(
    matched.len(),
    3,
    "client should have hit a fixture 3 times (success or error — both count)"
);
```

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

First-match-wins ordering matters here: the 429 fixture must come
*before* the success fixture so the initial request hits it. After the
scenario advances to `"failed"`, the first fixture no longer matches
(its `required_state` is unset / empty) and the third fixture takes
over.

```rust
use llmposter::{Fixture, RequestOutcome, ServerBuilder};

#[tokio::test]
async fn test_client_retries_on_429() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        // First hit: rate-limit the client and advance the scenario.
        .fixture(
            Fixture::new()
                .with_error(429, "Rate limited")
                .with_scenario("retry", Some(""), Some("failed")),
        )
        // Retry (scenario is now "failed"): serve the success response.
        .fixture(
            Fixture::new()
                .respond_with_content("success after retry")
                .with_scenario("retry", Some("failed"), Some("done")),
        )
        .build()
        .await?;

    // ... client with retry logic sends requests ...

    // After test, verify retry behavior. `was_matched()` is true for
    // BOTH the 429 error fixture AND the success fixture — Matched
    // means "a fixture was selected", not "HTTP 200".
    let reqs = server.get_requests();
    assert!(reqs.len() >= 2, "expected at least 1 retry, got {}", reqs.len());
    assert!(reqs.iter().all(|r| r.outcome == RequestOutcome::Matched));

    Ok(())
}
```

## Notes

- Requests are captured for ALL endpoints (LLM routes, `/code/{status}`, etc.) as of v0.4.5.
- Non-matched outcomes (malformed JSON, failed extraction, auth rejection, no-match) are now visible via the `outcome` field.
- The `body` field is the raw JSON string, not parsed. Use `serde_json::from_str` to inspect.
- `body` is empty for `GET /code/{status}` hits and auth-rejected requests — auth rejection captures the path and outcome, not the request body.
- Use `server.reset()` to clear captures between test phases.
