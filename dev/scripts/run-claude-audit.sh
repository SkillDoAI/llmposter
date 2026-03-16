#!/usr/bin/env bash
# Run Claude CLI audit (read-only, non-interactive)
# Usage: ./dev/scripts/run-claude-audit.sh [repo_path]
set -euo pipefail

REPO="${1:-$(cd "$(dirname "$0")/../.." && pwd)}"
OUTPUT="/tmp/claude-audit-$(date +%Y%m%d-%H%M%S).md"

cd "$REPO"

claude -p \
  --no-session-persistence \
  --model opus \
  --allowedTools "Read Glob Grep Bash(git:*) Bash(cargo:*) Bash(wc:*) Bash(head:*) Bash(tail:*)" \
  --output-format text \
  - <<'PROMPT' > "$OUTPUT"
You are a senior Rust engineer and application security specialist auditing a Rust crate called "llmposter" — a mock LLM API server providing fixture-driven, deterministic responses for testing.

Read BACKLOG.md first so you do not re-report tracked items or known-by-design decisions.
Read CLAUDE.md for project conventions and known gotchas.

Perform a full read-only audit of the repository. Focus on:

1. **Security** — injection vectors, unsafe input handling, credential exposure, YARA/lint bypass paths, container escape risks, prompt injection in generated content
2. **Correctness** — logic errors, off-by-one, race conditions, error handling that swallows failures, state that can desync
3. **Reliability** — panic paths in non-test code, unwrap() on fallible operations, timeout handling gaps, retry logic edge cases
4. **Rust-specific** — lifetime issues, unnecessary clones, missing Send/Sync bounds, clippy-suppressible patterns, unsafe usage
5. **Silent failure modes** — places where errors are logged but not propagated, where defaults mask broken state, where degraded operation is invisible to the caller
6. **Architecture** — dead code from recent refactors, unused imports, orphaned modules, abstraction leaks

Constraints:
- Do NOT make code changes. Read-only audit.
- Do NOT re-report items already tracked in BACKLOG.md "Known Issues" section.
- Sort findings by severity: P1 (must fix before release) → P2 (should fix soon) → P3 (nit/improvement).
- Include absolute file paths and line numbers for each finding.
- For each finding, state: what the issue is, why it matters, and a one-line fix suggestion.
- Be concise. No preamble. Findings only.
PROMPT

echo ""
echo "Audit written to: $OUTPUT"
