# v0.3.4 — Bug Fixes, CLI Testability, OpenAI 100% Spec Compliance

> **Goal:** Fix 3 security/correctness bugs, make CLI output testable, and achieve 100% OpenAI Chat Completions spec compliance.

**Architecture:** TDD throughout. Bug fixes are surgical changes to existing modules. CLI testability adds a writer parameter. OpenAI compliance is a deep audit against the real API spec with golden struct validation.

**Tech Stack:** Rust, serde, tokio (runtime deps). reqwest, serde_json (test-only dev deps). No new dependencies added.

---

## Execution Order

1. RegexBuilder size limit (P1) — independent, no interactions
2. Tool-call fixture argument validation (P2) — independent, no interactions
3. CLI output testability — independent, no interactions
4. Responses API streaming protocol (P1) — touches `src/format/responses.rs`, `src/handler/responses.rs`, `tests/spec/responses.rs`
5. OpenAI 100% spec compliance — touches `src/format/openai.rs`, `src/handler/openai.rs`, `tests/spec/openai.rs`, `tests/spec/types/openai.rs`. Builds on established patterns, no interaction with #4.

Items 1-3 can be parallelized. Items 4-5 are independent of each other.

---

## 1. RegexBuilder size limit (P1 bug fix)

**Files:** Modify `src/fixture.rs`. Test in `src/fixture.rs` (unit tests).

**Problem:** `Regex::new()` has no size limit on compiled DFA. Malicious fixtures could OOM.

**Fix:** Use `RegexBuilder::new().size_limit(1 << 20).build()` in both:
- `validate()` path (line ~248, regex pre-compilation)
- Fallback `is_match()` path (line ~45, runtime compilation)

**Tests:**
- Oversized regex pattern rejected at validation time
- Oversized regex in fallback `is_match()` path returns false (not panic/OOM)

---

## 2. Tool-call fixture argument validation (P2 bug fix)

**Files:** Modify `src/fixture.rs`. Test in `src/fixture.rs` (unit tests).

**Problem:** Fixture validation only checks `tool_calls` is non-empty. Non-object arguments load fine but produce invalid responses for Anthropic/Gemini.

**Fix:** In `validate()`, reject `tool_call.arguments` that aren't JSON objects.

**Tests:**
- Fixture with scalar arguments (`"hello"`) rejected at load time
- Fixture with array arguments (`[1,2,3]`) rejected at load time
- Fixture with object arguments (`{"key": "val"}`) passes validation

---

## 3. CLI output testability

**Files:** Modify `src/cli.rs`, `src/main.rs`. Extend `tests/cli_test.rs`.

**Problem:** `eprintln!` calls aren't capturable in tests.

**Fix:** Add `run_with_output(cli: &Cli, writer: &mut dyn Write + Send)` that takes a writer. Keep existing `run()` as a convenience wrapper that passes `stderr()`. All `eprintln!` calls in the function body become `writeln!(writer, ...)`.

Note: Since `run()` is async, the writer must be `Send`. Using `&mut dyn Write + Send` avoids the `impl Write` non-Send issue. The writer is only used before `.await` points (all output happens before server startup or during validate-and-exit), so this is safe.

**Tests (extend `tests/cli_test.rs`):**
- Validate mode outputs "Validated N fixtures successfully"
- Server mode outputs "llmposter listening on" and "Press Ctrl+C to stop"
- Error mode outputs error message

---

## 4. Responses API streaming protocol (P1 bug fix)

**Files:** Modify `src/format/responses.rs`, `src/handler/responses.rs`. Modify `tests/spec/responses.rs`, `tests/spec/types/responses.rs`.

**Problem:** Streaming events don't match the real OpenAI Responses streaming protocol:
- `response.created` is a flattened response object; real API wraps in `{"type":"response.created","response":{...}}`
- `response.completed` same issue — needs `{"type":"response.completed","response":{...}}`
- `response.output_item.added` removes `arguments` from tool-call items
- Text streaming events (`response.output_text.delta`) missing `output_index` and `content_index` correlation fields
- `response.done` should include `{"type":"response.done"}` (already correct)

**Fix:** Rebuild `build_stream_events()` and `build_tool_call_stream_frames()` to emit correctly structured events per https://platform.openai.com/docs/api-reference/responses-streaming.

**Tests:** Replace substring-matching streaming tests with golden struct deserialization:
- Each event type gets a spec struct in `tests/spec/types/responses.rs`
- Tests parse each SSE event and deserialize into the correct struct
- Verify event ordering: `response.created` → `response.in_progress` → deltas → `response.completed` (no `response.done` — `response.completed` is the terminal event per spec)

---

## 5. OpenAI Chat Completions 100% spec compliance

**Files:** Modify `src/format/openai.rs`, `src/handler/openai.rs`. Extend `tests/spec/openai.rs`, `tests/spec/types/openai.rs`.

**Known gaps to investigate and fix (based on prior Greptile/Codex findings + spec review):**

- **Error response shapes**: 400/401/429/500/503 should match OpenAI's `{"error":{"message":"...","type":"...","param":null,"code":"..."}}`  format. Currently uses generic shape.
- **Streaming stop chunk delta**: Should be `"delta":{}` (empty object), verify `skip_serializing_if` behavior produces this
- **`content: null` on role chunk**: Real API sends `"content": null` on the initial role-only chunk specifically. Currently omitted via `skip_serializing_if`.
- **Multiple tool calls**: Verify streaming with 2+ tool calls produces correct `index` values
- **Empty content responses**: Verify `content: ""` vs `content: null` handling
- **`id` field format**: Verify `chatcmpl-` prefix is consistent across streaming and non-streaming

**Approach:** Fetch the real spec, enumerate every field, write tests for each gap, fix production code. The exact list of changes will be discovered during the audit — if any gap is large enough to warrant its own PR, it will be split out.

---

## Quality Gates

- All tests pass (maintain 98%+ coverage)
- `cargo clippy -- -D warnings` clean
- `cargo fmt` clean
- Run Claude + Codex audits before committing, address findings, re-run
- CHANGELOG.md updated for v0.3.4
- PR opened (not draft) for CodeRabbit/Greptile review
- Target: Greptile 5/5, CodeRabbit approved, all conversations resolved
- 3-hour quiet period with no new CodeRabbit/Greptile findings after all benchmarks met

---

## Success Criteria

- RegexBuilder rejects oversized patterns in both paths
- Non-object tool-call arguments rejected at fixture load time
- Responses API streaming events match real protocol structure
- CLI output capturable and tested via writer parameter
- OpenAI golden structs cover 100% of documented response fields
- Every OpenAI behavioral contract has a spec test
- No regressions in existing 323 tests
- CHANGELOG.md updated
