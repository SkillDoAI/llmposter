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

Logs to stderr:
```text
[llmposter] POST /v1/chat/completions → fixture matched
[llmposter] POST /v1/messages → no match (model='claude-3', msg='hello...' (5 chars))
```
