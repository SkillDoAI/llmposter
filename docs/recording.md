# Record & Replay (VCR)

llmposter can act as a recording proxy: point your client at it with the client's own real API key, and every response from the real provider is saved as an ordinary replayable fixture. Run the suite once against the real API, then replay forever — no keys, no network, no flakes.

## Modes (v0.5.0+, `record` feature)

Set with `--vcr-mode <MODE>` on the CLI or `ServerBuilder::vcr_mode(VcrMode)` in library code. Requires the `record` Cargo feature (enabled by default).

| Mode | Fixture matching | Upstream | What gets recorded |
|------|------------------|----------|--------------------|
| `replay` (default) | Serves fixtures only | Never contacted | Nothing — identical to pre-0.5.0 behavior |
| `record` | Existing fixtures are ignored | Every request is forwarded | Every extractable 2xx response |
| `record-on-miss` | Fixture matches served locally | Only unmatched requests are forwarded | Extractable 2xx responses for misses |

`record-on-miss` is the everyday mode: hand-written fixtures and previously recorded entries keep serving locally, and only genuinely new prompts cost real tokens.

Recording covers all six LLM routes: `/v1/chat/completions`, `/v1/messages`, Gemini `generateContent` and `streamGenerateContent` (SSE via `?alt=sse`), `/v1/responses`, `/v1/completions`, and `/v1/embeddings`.

## Quick start

```bash
# 1. Record once — the client keeps using its own real API key,
#    responses land in fixtures/recorded.yaml
llmposter --fixtures fixtures/ --vcr-mode record-on-miss

# ... run your suite pointed at http://127.0.0.1:2112 ...

# 2. Replay forever — same command, default mode. The directory scan
#    picks up recorded.yaml automatically. No key, no network.
llmposter --fixtures fixtures/
```

llmposter itself takes no key configuration — the client sends its real `Authorization` / `x-api-key` / `x-goog-api-key` header as usual, and record mode forwards it to the upstream provider for that one request. Keys are never written to disk (see [Security & threat model](#security--threat-model)).

The cassette is a plain fixtures file. You can also replay it directly, in any mode, with no `record` feature involved:

```bash
llmposter --fixtures fixtures/recorded.yaml
```

Library equivalent:

```rust,no_run
use llmposter::{ServerBuilder, VcrMode};

let server = ServerBuilder::new()
    .vcr_mode(VcrMode::RecordOnMiss)
    .record_file("tests/fixtures/recorded.yaml")
    .build()
    .await?;
# Ok::<_, Box<dyn std::error::Error>>(())
```

## How recording works

A forwarded 2xx response is *extracted* into the minimal fixture schema — not stored as a raw HTTP transcript:

```yaml
fixtures:
  - match:
      user_message: "What is the capital of France?"
      model: "gpt-4o"
    provider: openai
    priority: -1
    response:
      content: "The capital of France is Paris."
      finish_reason: stop
```

Consequences of that design:

- **Headers and IDs are never persisted — by construction.** The recorded-fixture schema has no header, request-ID, or timestamp fields, so API keys, `set-cookie` values, and other response metadata *cannot* end up in a cassette. There is nothing to scrub. Response content can still contain secrets, so use `--redact` when needed.
- **`priority: -1` on every entry.** Hand-written fixtures default to priority `0`, so they always win over recordings. Override a recording by writing a normal fixture for the same prompt — no need to touch the cassette.
- **Provider pinning.** Each entry carries the `provider:` that recorded it, so an OpenAI recording never leaks into an Anthropic test.
- **Hand-editable.** Entries are ordinary fixtures; edit content, add `streaming:` blocks, or delete entries freely. The file header says as much.

### Cassette file

- Location: `--record-file <PATH>` / `.record_file(path)`. Defaults to `recorded.yaml` inside a `--fixtures` directory, next to a `--fixtures` file, or `./recorded.yaml` when the library builder loaded no file sources.
- Created at startup in a pristine `fixtures: []` state, with mode `0600` on Unix (recorded content can be sensitive).
- Hot-reload aware: on reload (SIGHUP / `--watch`) the cassette is re-read along with the other fixture sources. A cassette outside any directory source is tracked as the last reload source; one inside a `--fixtures` directory loads wherever the directory scan's alphabetical order puts it. Load order doesn't matter for precedence — hand-written fixtures win because recorded entries carry `priority: -1`.
- New recordings are also spliced into the live fixture set in memory, so in `record-on-miss` mode a prompt recorded once replays locally for the rest of the run — even before any reload. (`record` mode ignores fixtures for the whole run — see [Modes](#modes-v050-record-feature).)

### Dedupe and append idempotency

Recording is append-idempotent across runs. The dedupe key is the `(provider, model, user_message)` triple, and the in-memory dedupe set is seeded at startup from every loaded fixture carrying `priority: -1` — whichever file it lives in, not just the cassette. Re-running record mode against the same cassette forwards repeat prompts (in `record` mode) or serves them locally (in `record-on-miss`) but never appends duplicates.

The flip side: a recorded response is pinned until you remove it. To re-record a stale prompt, delete its entry from the cassette (or delete the whole file) and run record mode again.

## Upstreams & overrides

| Routes | Default upstream | Override |
|--------|------------------|----------|
| `/v1/chat/completions`, `/v1/completions`, `/v1/embeddings`, `/v1/responses` | `https://api.openai.com` | `--proxy-openai` / `.proxy_openai(url)` |
| `/v1/messages` | `https://api.anthropic.com` | `--proxy-anthropic` / `.proxy_anthropic(url)` |
| `/v1beta/models/{model}:generateContent`, `:streamGenerateContent` | `https://generativelanguage.googleapis.com` | `--proxy-gemini` / `.proxy_gemini(url)` |

The OpenAI override covers every OpenAI-format route, including the Responses API — which is how llmposter records from any OpenAI-compatible backend:

```bash
# Record from a local Ollama instead of api.openai.com
llmposter --fixtures fixtures/ --vcr-mode record --proxy-openai http://localhost:11434
```

Override URLs must be `http://` or `https://`, must not embed credentials (`user:pass@` is rejected at startup — upstream auth comes from the client's own forwarded headers), and are validated when a record mode is active. Plain `http://` to a non-loopback host triggers a loud stderr warning — real API keys would transit in cleartext. The `--proxy-*` flags have no effect in `replay` mode.

## Streaming

SSE responses are recorded with a true **stream-through tee**: upstream chunks are relayed to your client the moment they arrive — no buffering delay, byte-for-byte — while a background task keeps a copy. When the stream ends cleanly, the copy is reassembled into a single fixture and persisted.

- **Truncated or errored streams are never recorded.** Reassembly requires the provider's completion sentinel (`data: [DONE]` for the OpenAI family, `message_stop` for Anthropic, a final `finishReason` for Gemini SSE, `response.completed` for the Responses API). A mid-stream transport error or a missing sentinel means pass-through only.
- Both SSE line-ending dialects (`\r\n` and `\n`) are handled — real providers send CRLF.
- The recording buffer is capped at 16 MB. The cap bounds only the *recording* — an oversized stream keeps relaying to the client in full; the recording is abandoned with a stderr note. If your client disconnects mid-stream, llmposter keeps draining the upstream (bounded by the same cap) so the recording can still complete.
- **Gemini JSON-array streaming is not recorded.** `streamGenerateContent` without `?alt=sse` returns a JSON array, which passes through unrecorded. Use `?alt=sse` (which most Gemini SDKs default to) to record Gemini streams.

Recorded streaming responses replay as regular fixtures — add a `streaming:` block to an entry if you want replayed chunking/latency behavior.

## Redaction

`--redact <REGEX>` (repeatable) / `.redact(pattern)` masks every match as `[REDACTED]` in recorded response **content and tool-call arguments** before they are written to the cassette:

```bash
llmposter --fixtures fixtures/ --vcr-mode record \
  --redact 'sk-[A-Za-z0-9]+' \
  --redact '\b\d{3}-\d{2}-\d{4}\b'
```

The match keys (`user_message`, `model`) are deliberately **not** redacted — masking them would change what the fixture matches on and break replay. If your prompts themselves contain secrets, don't record them.

Patterns are compiled and validated at startup when a record mode is active; `--redact` is inert in `replay` mode.

## Security & threat model

Record mode handles real API keys, so its defaults are deliberately restrictive.

**Loopback-only by default.** Record modes refuse to start on a non-loopback bind address. llmposter itself speaks plain HTTP; binding beyond loopback would expose forwarded keys on the network. Loopback HTTP never leaves the kernel, so on a single machine there is no wire to sniff. (`localhost` is trusted by name without resolution — the standard hosts-file trust assumption.) If you genuinely need remote clients to record through llmposter, pass `--allow-remote-record` / `.allow_remote_record(true)` **and front llmposter with your own TLS terminator** (nginx, Caddy, a cloud load balancer) so keys are encrypted in transit.

**Why no built-in TLS listener?** A self-signed certificate would force every SDK to disable certificate verification (`verify=False` and friends) to connect — training users to bypass TLS verification is a worse outcome than loopback-only HTTP. Loopback needs no transport encryption; remote setups deserve a real certificate, which your own terminator provides.

**Keys are forwarded, never persisted.** Exactly these request headers are forwarded upstream: `authorization`, `x-api-key`, `x-goog-api-key`, `anthropic-version`, `anthropic-beta`, `openai-organization`, `openai-project`, `content-type`. Everything else is dropped. The cassette schema has no header fields, so keys cannot be written to disk by construction.

**Hardened upstream client.** The recording HTTP client follows no redirects (a redirect could re-send auth headers to a different host), uses a 10-second connect timeout, and terminates TLS with rustls. Gemini's `?key=` query parameter is percent-encoded and only `alt`/`key` survive forwarding. On upstream failure, the full request URL — which can carry a Gemini `?key=` — is stripped from 502 error bodies and log lines; the upstream *base* URL is deliberately named for debuggability, which is also why proxy URLs containing credentials (`user:pass@`) are rejected at `build()`.

**Auth simulation is rejected.** `build()` fails if a record mode is combined with mock bearer-token auth — the client's real key must pass through to the upstream untouched, and a configured mock token would intercept it.

**Cassettes are `0600` on Unix.** Recorded response content can be sensitive even without headers. This applies to cassettes llmposter creates; a pre-existing cassette keeps its permissions — deliberate, since checked-in cassettes are shared artifacts.

**The debug UI is open in record mode.** The UI requires a bearer token when auth is enabled (see [Authentication → Debug UI Access](authentication.md#debug-ui-access)) — but record modes are incompatible with mock auth, so a record-mode UI is always unauthenticated. Captured request/response bodies in record mode are real traffic: keep `--ui` off — or strictly loopback-bound — on shared hosts.

**Response headers are stripped.** Only a small allowlist is relayed from the upstream response: `retry-after`, `x-request-id`, `request-id`, and the `x-ratelimit-*` / `anthropic-ratelimit-*` families (so client backoff and throttling logic see real values). Everything else — including `set-cookie` — is stripped. Status, `content-type`, and body are relayed byte-exact.

## Capture API notes

Requests proxied by record mode appear in the [capture API](request-capture.md) with `RequestOutcome::Recorded`, and `status_code` reflects the real upstream status. Same-run replays of an already-recorded prompt are ordinary `Matched` entries, so a record-then-replay sequence captures `[Recorded, Matched]` — `matched_requests()` / `assert_matched()` count only the replays.

Ordering caveat: the status is known from the response headers when the stream starts, but a streaming recording deliberately defers its capture push until the upstream stream *finishes* — so the entry reflects the completed upstream stream, not necessarily what the client received (a client that disconnects mid-stream can still yield a completed recording via the salvage drain). Under concurrency this means `Recorded` entries appear in completion order, not arrival order.

## Limitations

- **Multi-input embeddings are not recorded.** The fixture schema stores a single vector, so `/v1/embeddings` requests whose array input has more than one element (or whose responses carry base64/non-numeric vectors, e.g. `encoding_format: "base64"`) pass through unrecorded. String input and single-element array input with float vectors record normally.
- **Mixed text + tool-call responses record the tool calls only.** The fixture schema is `content` XOR `tool_calls`; a response that called tools replays most faithfully as a tool call.
- **Unextractable 2xx responses pass through unrecorded** with a stderr note — e.g. Anthropic thinking-only responses, Gemini safety-blocked candidates, or response shapes the extractor doesn't recognize. Your client still gets the real response.
- **Non-2xx responses are never recorded.** A 429 or 500 passes through byte-exact (with `retry-after` and rate-limit headers intact) but is not immortalized as a fixture.
- **Gemini JSON-array streaming** (no `?alt=sse`) passes through unrecorded — see [Streaming](#streaming).
- Recording covers exactly the six routes listed under [Modes](#modes-v050-record-feature). Any other OpenAI-shaped path would fall through to the chat-completions extractor internally and, in practice, pass through unrecorded.
