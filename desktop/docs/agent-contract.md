# Agent loop contract (backend ⇄ frontend)

Source of truth for the Tauri commands and events used by the agentic run in the desktop app.
All new struct fields are `snake_case` (serde default). Tauri command **argument** names are camelCase at the
`invoke` call site (Tauri converts them), e.g. `invoke("approve_actions", { runId, actionIds })`.

## Modes

- `target_mode = "script"` keeps using the legacy `send_ai_request` command (GAS). Nothing changes there.
- `target_mode = "direct"` uses the new agent loop below. The Yemot token never leaves Rust except to call2all.co.il.

## Commands

### `start_agent_run(payload: AgentRunPayload) -> Result<AgentRunStarted, String>`

```jsonc
// AgentRunPayload
{
  "provider": "claude" | "gemini" | "openai" | "groq" | "custom",
  "model": "regular" | "pro" | "<explicit model id>",
  "prompt": "user request text",
  "api_key": "provider key (BYOK)",
  "base_url": "",                 // optional custom endpoint/base for direct mode
  "yemot_token": "…",             // held privately in Rust; never serialized outward
  "auto_apply": false,            // true = mutating tools execute immediately and real results feed the model
  "include_tree": true            // append a ≤20-line "[מצב נוכחי]" root tree after the first user message
}
// AgentRunStarted
{ "run_id": "r_…" }
```

Returns immediately; the loop runs in the background and reports through events. A second concurrent run is rejected with an error string.

### `cancel_agent_run(runId: string) -> Result<(), String>`

Cancels between tool calls; never interrupts an in-flight Yemot write.

### `approve_actions(runId: string, actionIds: string[]) -> Result<ActionApplyResult[], String>`

Executes the selected proposed actions (grouped per extension, one `UpdateExtension` per path) and returns per-action results. Also emits `agent:action_applied` per action.

```jsonc
// ActionApplyResult
{ "action_id": "a_1", "ok": true, "message": "…", "params": [ { "key": "type", "value": "menu", "applied": true, "note": null } ] }
```

### `execute_yemot_actions(token, path, params: {key,value}[]) -> ExtensionUpdateResult` and `read_extension_config(token, path) -> ExtReadDto`

Provided by `yemot.rs` (see that module). Used by the UI for manual edits / diff preview outside a run.

## Events (all payloads carry `run_id`)

| Event | Payload |
|---|---|
| `agent:started` | `{ run_id, provider, model }` |
| `agent:turn_start` | `{ run_id, turn, max_turns }` |
| `agent:assistant_text` | `{ run_id, turn, text }` — the authoritative full text block, sent once the turn is complete |
| `agent:text_delta` | `{ run_id, delta }` — one batch of streamed text; `{ run_id, delta: "", reset: true }` discards the partial block before a retry |
| `agent:tool_started` | `{ run_id, tool_use_id, name, label }` — `label` is a short Hebrew description, e.g. `חיפוש ידע: תפריט` |
| `agent:tool_finished` | `{ run_id, tool_use_id, ok, summary, ms }` — `summary` ≤ 120 chars, never raw file content |
| `agent:action_proposed` | `{ run_id, action: ProposedAction }` |
| `agent:actions_proposed` | `{ run_id, actions: ProposedAction[] }` — emitted once when the model stops; the UI shows the approval list |
| `agent:action_applied` | `{ run_id, action_id, ok, message, params: ParamOutcome[] }` |
| `agent:retry` | `{ run_id, attempt, max, reason, wait_ms }` |
| `agent:finished` | `{ run_id, ok, stop, final_text, usage }` — `stop ∈ end_turn, max_turns, cancelled, error, truncated, refusal` |
| `agent:error` | `{ run_id, code, message }` — `code ∈ session_expired, auth, bad_request, network, provider, internal` |

### Streaming

Every provider streams its turn over SSE (Anthropic `stream: true`, OpenAI-compatible
`stream: true` + `stream_options.include_usage`, Gemini `streamGenerateContent?alt=sse`)
and the runner forwards the text as `agent:text_delta`, batched to at most one event
per ~60ms or ~40 characters. The loop itself is unchanged: it still receives one
complete response per turn (tool calls, usage, stop reason) once the stream ends.

Rules the UI relies on:

* Deltas carry **answer text only** — never thinking / thought summaries.
* `agent:text_delta` text is not valid markdown until the block closes, so the UI
  renders it escaped and only runs markdown once `agent:assistant_text` (or
  `agent:finished`) finalizes the block. `assistant_text` **replaces** the streamed
  text rather than appending, so a dropped delta cannot corrupt the transcript.
* A retry re-streams the turn, preceded by `{ delta: "", reset: true }`.
* If the endpoint answers the streaming request with a 4xx (a custom gateway with
  no SSE support), the turn is retried once without streaming — no deltas, the
  same `assistant_text` at the end.

```jsonc
// ProposedAction
{
  "id": "a_1",
  "tool_use_id": "toolu_…",
  "kind": "set_extension_params" | "upload_text_file",
  "path": "/3",                       // display form; Rust canonicalizes to ivr2:/3
  "params": [ { "key": "type", "value": "menu" } ],
  "contents": null,                   // for upload_text_file
  "reason": "הגדרת שלוחה 3 כתפריט בחירה",
  "risk": "low" | "overwrite" | "destructive",
  "exists": true,                     // extension existed at read time (false → will be created)
  "diff": [ { "key": "type", "before": "playfile", "after": "menu", "kind": "changed" | "new" | "unchanged" } ],
  "warnings": [ "מפתח לא מתועד: enter_idd" ]
}

// usage (agent:finished)
{
  "input_tokens": 0, "output_tokens": 0, "cache_read_tokens": 0, "cache_write_tokens": 0,
  "turns": 0, "tool_calls": 0, "elapsed_ms": 0,
  "cache_hit_pct": 0.0,               // cache_read / (input + cache_read + cache_write)
  "cost_usd": null                    // null when the model is unknown to the price table
}
```

## Settings persisted by the frontend (localStorage keys already in use plus)

- `autoApply` (bool, default false)
- `includeTree` (bool, default true)

## Frontend behaviour

1. On submit in direct mode: call `start_agent_run`, store `run_id`, subscribe to events filtered by `run_id`, show a step list (turn header, tool rows with spinner/✓/✗ + summary, assistant text).
2. On `agent:actions_proposed`: populate the existing approval list (`parsedActions` shape: one row per action with checkbox; expandable diff rows). "בצע" calls `approve_actions`.
3. On `agent:finished`: hide spinner, show a cost line `עלות: $x · מטמון: y% · n סבבים · t שנ׳` (omit cost when null).
4. On `agent:error` with `session_expired`: open the existing MFA/login modal; the user re-runs.
5. Cancel button calls `cancel_agent_run`.
