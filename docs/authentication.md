# Authentication

llmposter supports bearer token enforcement on LLM endpoints, with optional OAuth 2.0 server integration via `oauth-mock`.

Auth is **off by default** — all existing code works without changes.

## Bearer Token Enforcement

Enable auth and register valid tokens:

```rust
let server = ServerBuilder::new()
    .with_bearer_token("test-token-123")          // unlimited uses
    .with_bearer_token_uses("short-lived", 1)     // expires after 1 LLM request
    .fixture(Fixture::new().respond_with_content("hello"))
    .build().await.unwrap();
```

Requests to LLM endpoints (`/v1/chat/completions`, `/v1/messages`, `/v1beta/models/*`, `/v1/responses`) must include a valid `Authorization: Bearer <token>` header. Invalid or missing tokens get a provider-specific 401 response with `WWW-Authenticate: Bearer realm="api"`.

### Provider-Specific 401 Responses

| Provider | Endpoint | Error Format |
|----------|----------|-------------|
| OpenAI | `/v1/chat/completions` | `{"error":{"type":"authentication_error","code":"invalid_api_key",...}}` |
| Anthropic | `/v1/messages` | `{"type":"error","error":{"type":"authentication_error",...}}` |
| Gemini | `/v1beta/models/*` | `{"error":{"code":401,"status":"UNAUTHENTICATED",...}}` |
| Responses | `/v1/responses` | Same as OpenAI |

### Token Expiry (`expires_after_uses`)

Tokens registered with `with_bearer_token_uses(token, N)` become invalid after exactly N LLM requests. This is deterministic — no real-time clocks involved. Perfect for testing refresh flows:

```rust
let server = ServerBuilder::new()
    .with_bearer_token_uses("expiring-token", 1)
    .fixture(Fixture::new().respond_with_content("hello"))
    .build().await.unwrap();

// First request: 200 OK
// Second request: 401 Unauthorized (token exhausted)
```

### Auth-Only Mode

`with_auth(true)` enables enforcement without registering tokens. Useful when combined with OAuth:

```rust
let server = ServerBuilder::new()
    .with_auth(true)
    // No bearer tokens — all auth comes from OAuth
    .with_oauth_defaults()
    .fixture(Fixture::new().respond_with_content("hello"))
    .build().await.unwrap();
```

## OAuth 2.0 Mock Server

llmposter integrates [`oauth-mock`](https://crates.io/crates/oauth-mock) to provide a full OAuth 2.0 server on a separate port. The OAuth server supports:

- **PKCE authorization code flow**
- **Device code flow** (with `approve_device_code` API)
- **Token refresh**
- **Token revocation**
- **OIDC discovery** (`.well-known/openid-configuration`)
- **JWKS endpoint** (`/jwks.json`)
- **Token introspection** (`/introspect`)

Tokens issued by the OAuth server are **automatically valid** on all LLM endpoints.

### Quick Start

```rust
let server = ServerBuilder::new()
    .with_oauth_defaults()  // spawns OAuth server on separate port
    .fixture(Fixture::new().respond_with_content("hello"))
    .build().await.unwrap();

// Two servers running:
let llm_url = server.url();                    // e.g. http://127.0.0.1:12345
let oauth_url = server.oauth_url().unwrap();   // e.g. http://127.0.0.1:54321

// Point your client's token_url at oauth_url, base_url at llm_url
```

### Custom OAuth Configuration

```rust
use llmposter::server::OAuthConfig;

let server = ServerBuilder::new()
    .with_oauth(OAuthConfig {
        client_id: "my-app".to_string(),
        client_secret: "my-secret".to_string(),
        redirect_uris: vec!["http://localhost:8080/callback".to_string()],
        scopes: vec!["openid".to_string(), "profile".to_string()],
    })
    .fixture(Fixture::new().respond_with_content("hello"))
    .build().await.unwrap();
```

### Getting Tokens

Use the OAuth server's token endpoint to obtain access tokens:

```rust
let client = reqwest::Client::new();
let (client_id, client_secret) = server.oauth_client_credentials().await.unwrap();

let resp = client.post(format!("{}/token", oauth_url))
    .form(&[
        ("grant_type", "client_credentials"),
        ("client_id", client_id.as_str()),
        ("client_secret", client_secret.as_str()),
    ])
    .send().await.unwrap();

let body: serde_json::Value = resp.json().await.unwrap();
let access_token = body["access_token"].as_str().unwrap();

// Use on LLM endpoint:
client.post(format!("{}/v1/chat/completions", llm_url))
    .header("Authorization", format!("Bearer {}", access_token))
    .json(&request_body)
    .send().await.unwrap();
```

### Device Code Flow

```rust
let server = ServerBuilder::new()
    .with_oauth_defaults()
    .fixture(Fixture::new().respond_with_content("hello"))
    .build().await.unwrap();

// Your client starts the device code flow...
// Then approve it from the test:
server.approve_device_code("USER-CODE-123").await.unwrap();
```

### Token Lifecycle Testing

Combine hardcoded tokens with OAuth for full lifecycle tests:

```rust
let server = ServerBuilder::new()
    .with_bearer_token_uses("initial-token", 1)  // expires after 1 use
    .with_oauth_defaults()                         // refresh via OAuth
    .fixture(Fixture::new().respond_with_content("hello"))
    .build().await.unwrap();

// 1. Use initial token → 200
// 2. Use initial token again → 401 (exhausted, won't fall through to OAuth)
// 3. Get new token from OAuth → use on LLM endpoint → 200
```

## Feature Flag

OAuth support is behind the `oauth` feature (enabled by default):

```toml
# With OAuth (default)
llmposter = "0.4"

# Without OAuth (smaller binary, fewer deps)
llmposter = { version = "0.4", default-features = false }
```

Bearer token enforcement works with or without the `oauth` feature.
