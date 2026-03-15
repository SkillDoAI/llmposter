# CLAUDE.md

Behavioral guidelines for the llmposter project. Merge with general coding instincts as needed.

## 1. Think Before Coding

- State assumptions explicitly. If uncertain, ask.
- If multiple interpretations exist, present them — don't pick silently.
- If a simpler approach exists, say so. Push back when warranted.

## 2. Simplicity First

- Minimum code that solves the problem. Nothing speculative.
- No features beyond what was asked.
- No abstractions for single-use code.
- If you write 200 lines and it could be 50, rewrite it.

## 3. Surgical Changes

- Don't "improve" adjacent code, comments, or formatting.
- Match existing style.
- Remove only imports/variables/functions that YOUR changes made unused.

## 4. Goal-Driven Execution

- Transform tasks into verifiable goals.
- For multi-step tasks, state a brief plan with verify steps.

---

## Project Overview

**llmposter** is a Rust crate + CLI for mocking LLM API endpoints. It's an impostor pretending to be a real LLM — fixture-driven, deterministic responses for testing.

### What It Does

- Speaks 4 LLM API response formats: OpenAI Chat Completions, Anthropic Messages, Gemini generateContent, OpenAI Responses API
- Streaming (SSE) and non-streaming for each format
- Fixture-driven: YAML files with canned responses, matched by model/prompt
- Failure simulation: truncated bodies, connection drops, rate limits (429), server errors (500/503)
- Usable as an in-process Rust library (`cargo add llmposter --dev`) or standalone binary

### Architecture

```
src/
  lib.rs              # Public API: server builder, fixture types, re-exports
  main.rs             # CLI binary (clap) — thin wrapper around lib
  fixture.rs          # YAML parsing, match logic (first-match-wins)
  server.rs           # axum router setup, shared state
  stream.rs           # SSE chunker — splits content into data: frames
  failure.rs          # Error responses, truncation, disconnect, latency, corruption
  handler/
    mod.rs            # Shared handler utilities, re-exports
    openai.rs         # POST /v1/chat/completions
    anthropic.rs      # POST /v1/messages
    gemini.rs         # POST /v1beta/models/{model}:generateContent
    responses.rs      # POST /v1/responses
  format/
    mod.rs            # Shared traits/types, re-exports
    openai.rs         # OpenAI Chat Completions request/response structs
    anthropic.rs      # Anthropic Messages API request/response structs
    gemini.rs         # Gemini generateContent request/response structs
    responses.rs      # OpenAI Responses API request/response structs
```

- `axum`-based HTTP server (on `hyper` — HTTP/1.1 + HTTP/2 native)
- Fixture loader reads YAML files, matches requests by model name or prompt content (substring or regex)
- Response formatters generate provider-specific JSON shapes per endpoint
- SSE streamer chunks responses into `data:` frames with configurable timing
- Failure modes: `truncate_after_chunks`, `disconnect_after_ms`, HTTP error codes, body corruption, latency injection

### Routing

No provider prefix — real API paths are already unique:
- `/v1/messages` → Anthropic
- `/v1/chat/completions` → OpenAI
- `/v1/responses` → OpenAI Responses API
- `/v1beta/models/{model}:generateContent` → Gemini

Clients just swap base URL to `http://127.0.0.1:{port}`. No path changes needed.

### Fixture Format (YAML)

```yaml
fixtures:
  # Simple text response — works for any provider endpoint
  - match:
      user_message: "stock price of AAPL"    # substring match (default)
    response:
      content: "The current stock price of AAPL is $150.42"

  # Regex match with streaming config
  - match:
      user_message:
        regex: "stock price of \\w+"
      model: "claude-sonnet-4-6"
    response:
      content: "I can help with stock prices."
    streaming:
      latency: 50       # ms between SSE chunks
      chunk_size: 20     # chars per chunk

  # Tool call response
  - match:
      user_message: "what's the weather"
    response:
      tool_calls:
        - name: get_weather
          arguments:
            location: "San Francisco"
            unit: "celsius"

  # Error simulation
  - match:
      model: "fail-model"
    error:
      status: 429
      message: "Rate limit exceeded"

  # Failure simulation (streaming)
  - match:
      user_message: "long response"
    response:
      content: "This will get cut off mid-stream..."
    failure:
      truncate_after_chunks: 3
      # disconnect_after_ms: 500
      # corrupt_body: true       (returns "overloaded" text)
      # latency_ms: 5000         (slow response)

  # Provider-specific override (optional, rarely needed)
  - match:
      user_message: "specific format"
    provider: anthropic
    response:
      content: "Provider-specific response"
      stop_reason: end_turn
```

- First-match-wins ordering
- Provider-agnostic by default; `provider` field available when format-specific fields needed
- `error` = HTTP error code response; `failure` = network/streaming simulation on valid response

### Key Design Decisions

- **Dual-target crate** — `lib.rs` + `main.rs`. Library for in-process `#[tokio::test]`, binary for standalone/CI use.
- **YAML fixtures, own format** — not llmock's JSON. Easier to author, supports comments, no trailing-comma pain.
- **Write from API specs + skilldo's client_impl.rs** — we know the response formats from parsing them. Reverse the direction.
- **Validate against our own clients** — if skilldo's Anthropic/OpenAI/Gemini clients accept the mock output, it's correct.
- **Inspired by llmock, not a copy** — different language, different fixture format, own test suite. Credit in README where appropriate.

---

## Environment

- API keys: `source ~/.openai` and `source ~/.anthropic` (both are `export VAR=value` format)
- This is a Rust project. Always use `cargo test`, `cargo clippy`, and `cargo build` for validation.

---

## Git & PR Workflow

- **Conventional commits**: Required. Format: `feat:`, `fix:`, `test:`, `docs:`, `refactor:`, `chore:`.
- **NEVER `git add -A` or `git add .`** — always stage files by name. Review untracked files before staging. Local docs (BACKLOG.md, BUILD.md, CLAUDE.md, AUDIT-PROMPT*.md) and dev artifacts must not be committed without explicit user approval.
- **Always create PRs as drafts** (`gh pr create --draft`). This prevents CodeRabbit from reviewing before the PR is clean. Mark ready for review only when the diff is final.
- **Commit/push timing** — weekdays during working hours (~9a-5p): ask before committing and pushing. Weekends and evenings: can commit and push freely, especially if user has given permission for the session. User may also grant blanket permission to open PRs or even merge if there are no greptile/coderabbit/coverage/audit issues.
- **Bedtime mode** — When the user says something like "do you remember how we worked last night?" or explicitly grants autonomous mode: push, open draft PR, run all audit scripts, fix findings, iterate with reviewers, mark ready for review — all without asking. Only stop to ask if something is genuinely blocked. This is the default for evening/weekend sessions once granted.
- **No git worktrees** — worktrees set `bare = true` on the main repo and break pre-commit/pre-push hooks. Just branch from main in the normal working directory.
- **No force pushes** — repo has force push protection. Always use regular `git push`, never `--force` or `--force-with-lease`.

---

## Quality Gates & Reviews

- **Always keep test coverage above 98%**. Never let new code drop coverage below 98%.
- **Greptile confidence score must be 5/5** — aim for maximum Greptile review confidence on every PR.
- **CodeRabbit must have no outstanding findings** — all CodeRabbit comments must be addressed in-thread. CodeRabbit should not be finding new issues on each review cycle.
- **Merge readiness** — both Greptile (5/5) and CodeRabbit (no new findings) must be clean, coverage ≥ 98%, CI green. After final audits, decide with user what to backlog vs finish.
- **"The 4 Horsemen" / "audit"** — Run ALL five reviewers on the uncommitted diff BEFORE committing: `/simplify` (reuse/quality/efficiency), Codex (architecture/security), Gemini (output quality/prompts), CodeRabbit (nits/style), Claude (`dev/scripts/run-claude-audit.sh` — Rust/security deep dive). User may say "roll the 4 horsemen" or "run an audit" — same thing, all 5 run. Fix P1/P2 findings before commit.
- **CHANGELOG.md is mandatory** — every PR gets a changelog entry, even QoL branches that don't bump the version.
- **Doc accuracy** — docs must match actual behavior. Don't document aspirational features as current.
- **Reply to review nits inline** — NEVER post standalone PR comments summarizing nit responses. Reply directly in the review thread where the comment lives. Always tag the bot by handle so it sees the reply and can resolve the thread: `@coderabbitai` for CodeRabbit, `@greptile-apps` for Greptile. Do this for ALL review bots, every time.
- **Do NOT resolve review threads yourself** — only the bot or the user should resolve threads. Reply in the thread, let the bot acknowledge and resolve. If the bot doesn't resolve, leave it open for the user.
- **Check for replies to your replies** — bots may respond to your fix confirmations with follow-up questions or new concerns. Monitor threads after replying and respond to any follow-ups. Use the GraphQL API with pagination (`first:100` then `last:20`) to find ALL unresolved threads — there may be more than 100.
- **Use GraphQL for thread management** — the REST API `/pulls/comments` endpoint paginates at 30 and doesn't reliably link replies on outdated diffs. Use the GraphQL `reviewThreads` query to find unresolved threads and `addPullRequestReviewThreadReply` to reply in-thread.
- **Audit scripts** at `dev/scripts/`: `run-gemini-audit.sh`, `run-codex-audit.sh`, `run-coderabbit.sh`, `run-claude-audit.sh`. Not committed. Run these directly — they are logged in and don't need user intervention.

### The 4 Horsemen (really 5)

| # | Auditor | Tool | Focus |
|---|---------|------|-------|
| 1 | /simplify | Skill (3 parallel agents) | Code reuse, quality, efficiency |
| 2 | Codex | `dev/scripts/run-codex-audit.sh` | Architecture, security |
| 3 | Gemini | `dev/scripts/run-gemini-audit.sh` | Output quality, prompts |
| 4 | CodeRabbit | `dev/scripts/run-coderabbit.sh` | Nits, style |
| 5 | Claude | `dev/scripts/run-claude-audit.sh` | Rust deep dive, security |

All free (CLI subscriptions), all find different classes of bugs. Run them all before committing.

---

## Testing Conventions

### TDD Workflow
- Write failing tests BEFORE implementation.
- Use AAA pattern: Arrange-Act-Assert.
- Test names describe behavior: `should_return_openai_format_for_chat_completions`.

### Running Tests
- `cargo test`, `cargo clippy`, `cargo build` for validation.
- Run the full test suite after changes to check for regressions.

---

## Model Preferences

- "sonnet" = `claude-sonnet-4-6` (Sonnet 4.6) until further notice
- "openai" = ChatGPT 5.2, or 5.3 if available
- NEVER default to older models like `claude-sonnet-4-20250514` (Sonnet 4)

---

## Dependencies

- Pin to specific known-working versions. Never use `@latest` without verifying.
- Core deps: `tokio`, `axum`, `serde`, `serde_json`, `serde_yaml`, `clap`. Keep it minimal.

---

## Working Style

- **MR = PR** — user may say "MR" (merge request) when they mean "PR" (pull request).
- When the user asks you to test or validate something, start executing immediately. Do not spend time exploring the codebase or asking clarifying questions unless truly blocked.
- When debugging, always check debug logs and error output the user points to FIRST before doing broad exploration.

---

## License

AGPL-3.0 — free to use, modify, and distribute. Can't close-source it.
