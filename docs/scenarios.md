# Stateful Scenarios

Scenarios enable multi-turn fixture matching via named state machines. A fixture can require a specific state to match, and advance the state after matching — enabling tool-call loops, retry sequences, and conversation progression.

## YAML Format

```yaml
fixtures:
  # Step 1: Agent asks about weather → server returns tool call
  - match:
      user_message: "weather"
    scenario:
      name: "weather-flow"
      required_state: ""           # match only when state is initial (empty)
      set_state: "tool_called"     # advance to this state
    response:
      tool_calls:
        - name: get_weather
          arguments:
            location: "Paris"

  # Step 2: Agent sends tool result → server returns text
  - match:
      user_message: "weather"
    scenario:
      name: "weather-flow"
      required_state: "tool_called"  # only match after step 1
      set_state: "done"
    response:
      content: "It's 22°C and sunny in Paris"
```

### Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Scenario identifier — shared across fixtures in the same flow |
| `required_state` | string | no | Only match when the scenario is in this state. Omit to match regardless of state. Use `""` (empty string) for initial/unset state. |
| `set_state` | string | no | Advance the scenario to this state after matching. Omit to leave state unchanged. |

## Builder API

```rust
use llmposter::{Fixture, ServerBuilder};
use llmposter::fixture::ToolCall;

#[tokio::test]
async fn test_tool_call_loop() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("weather")
                .respond_with_tool_calls(vec![ToolCall {
                    name: "get_weather".to_string(),
                    arguments: serde_json::json!({"location": "Paris"}),
                }])
                .with_scenario("weather-flow", Some(""), Some("tool_called")),
        )
        .fixture(
            Fixture::new()
                .match_user_message("weather")
                .respond_with_tool_calls(vec![])
                .respond_with_content("It's 22°C in Paris")
                .with_scenario("weather-flow", Some("tool_called"), Some("done")),
        )
        .build()
        .await?;

    // Query scenario state at any point
    assert_eq!(server.scenario_state("weather-flow"), None); // not yet entered

    // ... send requests ...

    assert_eq!(server.scenario_state("weather-flow"), Some("done".to_string()));
    Ok(())
}
```

### `Fixture::with_scenario(name, required_state, set_state)`

| Parameter | Type | Description |
|-----------|------|-------------|
| `name` | `&str` | Scenario name |
| `required_state` | `Option<&str>` | `None` = match regardless of state. `Some("")` = match only when unset. `Some("x")` = match only when state is `"x"`. |
| `set_state` | `Option<&str>` | `None` = don't change state. `Some("x")` = set state to `"x"` after match. |

## How Matching Works

1. Fixtures are evaluated using the standard [two-pass priority-sorted
   ordering](fixtures.md#ordering): non-catch-all fixtures sorted by
   descending `priority` first (file order as stable tiebreak), then
   catch-all fixtures in the same order. Scenario constraints are
   checked during this scan — they don't change the ordering, only
   whether a given fixture participates.
2. If a fixture has `scenario.required_state`, the match is skipped unless the scenario's current state matches.
3. After a fixture matches, if it has `scenario.set_state`, the scenario state is updated.
4. Fixtures without a `scenario` block are stateless — they always participate in matching regardless of any scenario state.

## Querying and Resetting State

```rust
// Check current state
let state = server.scenario_state("weather-flow");
assert_eq!(state, Some("tool_called".to_string()));

// Reset all scenarios and captured requests
server.reset();
assert_eq!(server.scenario_state("weather-flow"), None);
```

## Common Patterns

### Retry simulation

```yaml
fixtures:
  - match:
      user_message: "flaky"
    scenario:
      name: "retry"
      required_state: ""
      set_state: "failed_once"
    error:
      status: 429
      message: "Rate limited"

  - match:
      user_message: "flaky"
    scenario:
      name: "retry"
      required_state: "failed_once"
      set_state: "succeeded"
    response:
      content: "Success on retry"
```

### Conversation branching

```yaml
fixtures:
  - match:
      user_message: "start"
    scenario:
      name: "convo"
      set_state: "greeting"
    response:
      content: "Hello! How can I help?"

  - match:
      user_message: "help with code"
    scenario:
      name: "convo"
      required_state: "greeting"
      set_state: "coding"
    response:
      content: "Sure, what language?"
```

## Notes

- Scenario state is per-server-instance — each test gets its own state.
- State is stored in memory (not persisted). Use `server.reset()` to clear between sub-tests.
- Multiple independent scenarios can coexist — each has its own name and state.
- Empty string `""` is a valid state and is distinct from "unset" (`None`).
