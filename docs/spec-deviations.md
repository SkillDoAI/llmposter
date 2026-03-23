# Known Spec Deviations

llmposter aims for 100% API spec compliance. This page documents every known gap.

## OpenAI Chat Completions

### Role-only streaming chunk omits `content: null`

**Real API:** First streaming chunk sends `"content": null` explicitly alongside `"role": "assistant"`.

**llmposter:** Omits `content` entirely on the role-only chunk (via `skip_serializing_if`).

**Impact:** None. Every OpenAI SDK treats absent and `null` identically for `Option<String>` fields.

**Reason:** We can't selectively emit `null` on one chunk type while correctly omitting `content` on all other chunk types without a custom serializer. Zero practical benefit.

## All Providers

### Token counts are estimated

**Real APIs:** Return actual tokenizer-computed token counts.

**llmposter:** Uses a `bytes / 4` heuristic. Token counts are approximately correct but not exact. Assert they are positive and that `total == prompt + completion`, not specific values.

### Request fields silently ignored

llmposter accepts all valid request fields (`temperature`, `top_p`, `max_tokens`, `tools`, `metadata`, etc.) and silently ignores them. Only `model`, `messages`/`input`/`contents`, and `stream` are used for fixture matching.

This is intentional — your real client code can send any parameters without modification.
