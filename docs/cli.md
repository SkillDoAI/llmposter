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

Logs to stderr (user prompt truncated to 50 chars in verbose logs; not included in HTTP 404 response body):
```text
[llmposter] POST /v1/chat/completions → fixture matched
[llmposter] POST /v1/messages → no match (model='claude-3', msg='hello...' (5 chars))
```

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
