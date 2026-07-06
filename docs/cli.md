# CLI Reference

## Usage

```text
llmposter --fixtures <PATH> [OPTIONS]
```

## Options

| Flag | Description | Default |
|------|-------------|---------|
| `--fixtures <PATH>` | Path to a YAML fixture file or directory | Required |
| `--validate` | Validate fixtures and exit (no server) | Off |
| `--port <PORT>` | Port to listen on | `2112` |
| `--bind <ADDR>` | Bind address (IPv4 or IPv6) | `127.0.0.1` |
| `--verbose` | Log matched/unmatched requests to stderr | Off |
| `--watch` / `-w` | Hot-reload fixtures when files change (see [Hot Reload](#hot-reload)) | Off |
| `--capture-capacity <N>` | Maximum captured requests retained in memory; `0` disables retention | `1000` |
| `--diagnostics` | Include nearest-match diagnostics in 404 no-match responses (which fixture came closest, per-field pass/fail) | Off |
| `--ui` | Enable the embedded debug UI at `/ui` (requires the `ui` feature) | Off |
| `--vcr-mode <MODE>` | VCR mode: `replay` (fixtures only), `record` (proxy everything upstream, save responses as fixtures), or `record-on-miss` (proxy only unmatched requests). Requires the `record` feature. See [Record & Replay](recording.md). | `replay` |
| `--record-file <PATH>` | Cassette file for recorded fixtures | `recorded.yaml` inside a `--fixtures` directory, or next to a `--fixtures` file |
| `--proxy-openai <URL>` | Upstream override for OpenAI-format routes — chat, completions, embeddings, Responses API (vLLM, Ollama, gateways) | `https://api.openai.com` |
| `--proxy-anthropic <URL>` | Upstream override for `/v1/messages` | `https://api.anthropic.com` |
| `--proxy-gemini <URL>` | Upstream override for Gemini routes | `https://generativelanguage.googleapis.com` |
| `--redact <REGEX>` | Mask matches as `[REDACTED]` in recorded response content and tool-call arguments. Repeatable. | None |
| `--allow-remote-record` | Allow record modes on non-loopback binds — see the [threat model](recording.md#security--threat-model) before using | Off |

The `--proxy-*` and `--redact` flags have no effect unless `--vcr-mode` is
`record` or `record-on-miss`.

## Examples

### Start with a single fixture file

```bash
llmposter --fixtures fixtures.yaml
```

### Start with a directory of fixtures

```bash
llmposter --fixtures fixtures/
```

All `.yaml` and `.yml` files in the directory are loaded. Subdirectories are not recursed.

### Validate fixtures without starting

```bash
llmposter --fixtures fixtures/ --validate
```

Validates YAML syntax, fixture invariants (mutual exclusivity, required fields), and regex patterns. Exits with 0 on success, non-zero on error.

### Bind to all interfaces

```bash
llmposter --fixtures fixtures.yaml --bind 0.0.0.0 --port 8080
```

### IPv6

```bash
llmposter --fixtures fixtures.yaml --bind ::1
```

### Verbose logging

```bash
llmposter --fixtures fixtures.yaml --verbose
```

Logs to stderr. No-match lines deliberately log only the user message's
character count — never its content — so prompts can't leak into CI logs
or shared terminals (use the request capture API or the debug UI for the
full body):
```text
[llmposter] POST /v1/chat/completions → fixture matched
[llmposter] POST /v1/messages → no match (model='claude-3', msg len=5 chars)
```

### Bound request capture

```bash
llmposter --fixtures fixtures.yaml --capture-capacity 500
```

The CLI keeps the most recent 1000 captured requests by default so long-lived
servers do not grow memory without bound. Use `--capture-capacity 0` to
disable retained capture entries. The debug UI live feed still works when
the `ui` feature is enabled.

### Debug UI

```bash
cargo install llmposter --features ui
llmposter --fixtures fixtures.yaml --ui
```

When built with the optional `ui` feature, `--ui` serves an embedded debug
UI at `/ui` with a request inspector, live SSE feed, fixture list, and match
debugger. The published binary may omit this feature; if `--ui` is not present
in `llmposter --help`, install with `--features ui`.

### Record real responses, then replay

```bash
# Record: misses are forwarded to the real API (using the client's own
# key) and saved to fixtures/recorded.yaml
llmposter --fixtures fixtures/ --vcr-mode record-on-miss

# Replay: default mode — the directory scan picks up recorded.yaml
llmposter --fixtures fixtures/
```

See [Record & Replay](recording.md) for modes, upstream overrides,
redaction, and the security model.

## Hot Reload

llmposter can reload fixtures without restarting the server, so you can edit
a YAML file and have the changes picked up automatically by the running
process.

There are two reload triggers:

### `--watch` (file watcher)

```bash
llmposter --fixtures fixtures.yaml --watch
```

Watches the fixture file or directory. On change, re-reads and re-validates
all tracked sources; if validation succeeds the fixtures are atomically
swapped. If parsing or validation fails, the previous fixtures keep serving
and an error is logged to stderr — a partial edit or syntax error will
never take down the live server.

File-system events are debounced by ~250ms so editor "save as temp → rename"
sequences collapse into a single reload.

Requires the `watch` feature (enabled by default).

### `SIGHUP` (Unix only)

On Unix systems, `kill -HUP <pid>` always triggers a reload — even without
`--watch`. This matches traditional daemon conventions and gives you a
backstop for when you forget `--watch`:

```bash
# reload the fixtures file in a running llmposter
kill -HUP $(pgrep llmposter)
```

llmposter prints the exact command at startup:

```text
llmposter listening on http://127.0.0.1:2112
Send SIGHUP (kill -HUP 84500) to reload fixtures
Press Ctrl+C to stop
```

Same validation and fallback rules as `--watch` apply: invalid YAML or
invalid fixtures leave the server running the previously loaded set.
