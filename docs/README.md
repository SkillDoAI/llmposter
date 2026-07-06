# llmposter Documentation

## Table of Contents

- [Getting Started](getting-started.md) — Installation, first fixture, first test
- [Fixtures](fixtures.md) — YAML fixture format reference, matching rules, tool calls
- [Failure Simulation](failure-simulation.md) — Error codes, latency, truncation, disconnect
- [Authentication](authentication.md) — Bearer tokens, OAuth 2.0 mock server, token expiry
- [Stateful Scenarios](scenarios.md) — Multi-turn fixture matching and retry flows
- [Request Capture](request-capture.md) — Assert what your client sent
- [Record & Replay](recording.md) — Record real API responses as fixtures, replay offline
- [CLI Reference](cli.md) — Command-line usage, flags, validate mode
- [Library API](library.md) — Rust `ServerBuilder` API, programmatic fixtures, in-process testing
- [Spec Deviations](spec-deviations.md) — Known gaps from real APIs

### Provider Guides

Each provider doc covers: supported fields, streaming event sequence, error shapes, and spec compliance notes.

- [OpenAI Chat Completions](providers/openai.md)
- [Anthropic Messages](providers/anthropic.md)
- [Gemini generateContent](providers/gemini.md)
- [OpenAI Responses API](providers/responses.md)
