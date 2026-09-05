# Agent loop contract (backend ⇄ frontend)

Source of truth for the Tauri commands and events used by the agentic run in the desktop app.
All new struct fields are `snake_case` (serde default). Tauri command **argument** names are camelCase at the
`invoke` call site (Tauri converts them), e.g. `invoke("approve_actions", { runId, actionIds })`.

## Product model

The app is an **editor for phone lines**, not a chat. One run = one **task** (משימה): the user describes what
should change, the loop reads the line, and every mutating step comes back as a **proposed change**
(שינוי מוצע) with a before/after diff — never a silent write. The user **approves** the proposals they want
(`approve_actions`), and applied changes land in the **change log** (יומן שינויים), each undoable while the
server state has not moved.

Refining a task (`continue_agent_run`) is a **refinement of the same task**, not a conversational turn: it
resumes the parent transcript and prepends the status of the parent's proposals (בוצע / בוטל / לא אושר).
Nothing here carries free-form conversation state; when a task cannot be refined (parent evicted, unfinished,
or past `CONTEXT_HARD_LIMIT`) the answer is a new task, not a longer thread.

User-facing strings follow this model — "משימה", "שינויים מוצעים", "יומן שינויים", "מבנה הקו" — and avoid
chat vocabulary ("צ'אט", "שיחה", "הודעה", "עוזר").

## Cost line

The cost shown after a task is an **estimate**, not a bill. It comes from the public list-price table in
`agent/pricing.rs`, frozen at `PRICE_LIST_DATE`, which `agent:finished` reports as `usage.price_list_date`
exactly when it reports a `cost_usd`. Free tiers, promotional rates and provider price changes are invisible
to the app, and a model absent from the table yields `cost_usd: null` (and no `price_list_date`) — the UI then
omits the cost from the line rather than showing a guess.

## Modes

- `target_mode = "script"` keeps using the legacy `send_ai_request` command (GAS). It POSTs a JSON body (prompt, optional model/key) to the script URL — never a query string, so the prompt and any personal API key never sit in a proxy or access log. The Yemot token is never part of this payload.
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
  "include_tree": true,           // append a ≤20-line "[מצב נוכחי]" root tree after the first user message
  "attachments": []               // optional (serde default); audio files the user picked for this task
}
// Attachment
{ "id": "f1", "name": "ברכה.mp3", "local_path": "C:/…/ברכה.mp3", "size": 20480, "mime": "audio/mpeg" }
// AgentRunStarted
{ "run_id": "r_…" }
```

Returns immediately; the loop runs in the background and reports through events. A second concurrent run is rejected with an error string.

**Attachments.** Validated at run start (an error rejects the whole run, nothing is emitted): the name must end in one of `wav, mp3, m4a, ogg, wma, aac`, a non-empty `mime` must start with `audio/`, the size must be ≤ 25 MiB (declared *and* on disk), the ids must be unique, and `local_path` must exist and be a regular file. The list appears in the first user message, after the `[מצב נוכחי]` tree block:

```
[קבצים מצורפים]
- f1: ברכה.mp3 (20 KB, audio/mpeg)
```

`local_path` never reaches the model: it names an `id` from that block, and `upload_audio_file` resolves the id against **this run's own attachment list** only.

### `continue_agent_run(payload: AgentRunPayload, parentRunId: string) -> Result<AgentRunStarted, String>`

Refines a finished task instead of starting over. The new run replays the parent's full transcript (the system blocks are regenerated from consts, so they are byte-identical and the provider's cached prefix still hits) and appends one user message: the status of the parent's proposals, then `payload.prompt`, then the `[קבצים מצורפים]` list when this continuation attached more files.

```
[מצב ההצעות הקודמות]
- a_1 /3: בוצע
- a_2 /4: בוטל
- a_3 /5: לא אושר
```

`בוצע` / `בוטל` / `לא אושר` come from the write state of the parent run, not from the proposal list — an id is `בוטל` once `undo_action` reversed it. The events are exactly a fresh run's (`agent:started` … `agent:finished`) under a **new** `run_id`. Errors (Hebrew, nothing started): the parent is not the run in the registry any more, the parent is still running, or the parent transcript alone already exceeds `CONTEXT_HARD_LIMIT` (`"המשימה ארוכה מדי להמשך, התחל משימה חדשה"`). `include_tree` is ignored — the tree is already in the transcript.

### `cancel_agent_run(runId: string) -> Result<(), String>`

Cancels between tool calls; never interrupts an in-flight Yemot write.

### `approve_actions(runId: string, actionIds: string[]) -> Result<ActionApplyResult[], String>`

Executes the selected proposed actions (grouped per extension, one `UpdateExtension` per path) and returns per-action results. Also emits `agent:action_applied` per action.

```jsonc
// ActionApplyResult
{
  "action_id": "a_1", "ok": true, "message": "…",
  "params": [ { "key": "type", "value": "menu", "applied": true, "note": null } ],
  "undo": { "action_id": "a_1", "kind": "set_extension_params", "path": "/3", "params": [ ["type", "playfile"] ], "contents": null }
  // `undo` is `null` when the action failed or cannot be undone (see below).
}
```

### `undo_action(runId: string, actionId: string) -> Result<ActionApplyResult, String>`

Reverses one already-applied action by writing back the values `approve_actions` recorded as `undo`. Re-reads the server first and refuses (`ok: false`, no write) if the state has moved since the apply — see "Approval and undo lifecycle" below.

### `list_applied_changes() -> Result<AppliedChange[], String>`

The change log ("יומן שינויים") of every run still retained in the write state, newest first. It is read from the write state rather than from the run handle, so it survives a page reload and a later run evicting the handle.

```jsonc
// AppliedChange
{
  "run_id": "r_…", "action_id": "a_1",
  "kind": "set_extension_params" | "upload_text_file" | "upload_audio_file",
  "path": "/1/2",                     // as shown on the action
  "display": "/1/2",                  // canonical path in display form
  "applied_at_ms": 1757000000000,
  "params": [ { "key": "type", "value": "menu" } ],
  "undo_available": true,             // false for audio uploads, failed undos and already-undone rows
  "undone": false,                    // set by undo_action; the row stays in the log
  "label": "עדכון 3 הגדרות ב-/1/2"     // short Hebrew row label ("העלאת קובץ 000.wav ל-/1", …)
}
```

Rows are pruned with the rest of the write state (50 runs / 1 hour, cleared on logout). Re-approving an id after an undo replaces its row instead of adding a second one.

### `execute_yemot_actions(token, path, params: {key,value}[]) -> ExtensionUpdateResult`

Provided by `yemot.rs` (see that module). Used only by legacy script mode to apply the parsed action list.

### `get_extension_tree(token, root, depth) -> Result<ExtTreeNode[], String>`

Read-only. Lists the extensions under `root` (`""` / `"/"` = the whole line) with `YemotClient::list_extensions` and nests the flat result by path. `depth` is clamped to 1..=2 — the full range `list_extensions` implements (`1` = the children of `root`, `2` = one level below them); a larger value never produced a deeper tree. A node whose direct parent is missing from the listing is attached to its nearest present ancestor, or to the top level when it has none.

```jsonc
// ExtTreeNode
{
  "path": "ivr2:/1/2",   // canonical
  "display": "/1/2",     // what the UI shows
  "ext_type": "menu",    // "" when the server reported none
  "title": "תפריט ראשי", // "" when the server reported none
  "children": []
}
```

### `read_extension(token, path) -> Result<ExtensionDetail, String>`

Read-only. `get_ext_ini_fresh` + `list_files` for one extension; never touches the write state.

```jsonc
// ExtensionDetail
{
  "path": "ivr2:/1/2", "display": "/1/2",
  "exists": true,                  // false = no ext.ini (the extension is not configured)
  "ext_type": "menu",              // null when there is no `type` key
  "params": [ { "key": "type", "value": "menu" } ],   // file order, comments dropped
  "raw": "type=menu\n",            // ext.ini text, capped at 64 KiB + a trailing "…[קוצץ]"
  "size": 214, "mtime": "01/01/2026 10:00",
  "files": [ { "name": "000.wav", "size": 12, "mtime": null, "kind": "audio" } ]
  // kind: "ini" (ext.ini) | "audio" (wav/mp3/m4a/ogg/wma/aac) | "text" (txt/ini/tts/csv/json/api) | "other"
}
```

Both commands fail closed on an empty token and render every failure with `yemot::render_error`, so an expired or unverified session arrives as an `Err` string prefixed `SESSION_EXPIRED: ` — the same code prefix the run loop maps to `agent:error { code: "session_expired" }`.

### Secrets: `secret_set(name, value)`, `secret_get(name) -> Option<String>`, `secret_delete(name)`

Provided by `secrets.rs`. Store the Yemot token and provider API keys in the OS credential store (Windows Credential Manager / macOS Keychain / Secret Service) under service name `ai-yemot`, never in `localStorage`. `name` must be one of a fixed allowlist (`yemot_token`, `api_key_claude`, `api_key_gemini`, `api_key_openai`, `api_key_groq`, `api_key_custom`, `custom_base_url`) — anything else is rejected before it reaches the keychain. `secret_set` with an empty value deletes the entry. A locked/unavailable credential store makes `secret_get` return `None` (the app still starts); the same condition on write or delete is a hard error.

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
  "kind": "set_extension_params" | "upload_text_file" | "upload_audio_file",
  "path": "/3",                       // display form; Rust canonicalizes to ivr2:/3
  "params": [ { "key": "type", "value": "menu" } ],
  "contents": null,                   // for upload_text_file
  "reason": "הגדרת שלוחה 3 כתפריט בחירה",
  "risk": "low" | "overwrite" | "destructive",
  "exists": true,                     // extension existed at read time (false → will be created)
  "diff": [ { "key": "type", "before": "playfile", "after": "menu", "kind": "changed" | "new" | "unchanged" } ],
  "warnings": [ "מפתח לא מתועד: enter_idd" ],
  "previous": null,                   // upload_text_file only: file contents before the write (null = file did not exist)
  "snapshot_hash": "true:1234"        // fingerprint of ext.ini / the file as read; approval refuses to write if it no longer matches
}

// ProposedAction, kind = "upload_audio_file"
{
  "id": "a_2", "kind": "upload_audio_file",
  "path": "ivr2:/1/000.wav",          // the destination file; must end in .wav (the system converts the source)
  "params": [ { "key": "file", "value": "ברכה.mp3" }, { "key": "size", "value": "20 KB" } ],
  "contents": null,
  "risk": "overwrite",                // "overwrite" when the destination already holds that file, else "low"
  "exists": true,
  "diff": [ { "key": "file", "before": "000.wav", "after": "ברכה.mp3", "kind": "changed" } ],
  "warnings": [ "לא ניתן לבטל העלאת שמע אוטומטית" ],   // only when the destination exists
  "previous": null, "snapshot_hash": null
}

// usage (agent:finished)
{
  "input_tokens": 0, "output_tokens": 0, "cache_read_tokens": 0, "cache_write_tokens": 0,
  "turns": 0, "tool_calls": 0, "elapsed_ms": 0,
  "cache_hit_pct": 0.0,               // cache_read / (input + cache_read + cache_write)
  "cost_usd": null,                   // null when the model is unknown to the price table
  "price_list_date": null             // "2026-09" — the public price list behind cost_usd;
                                      // non-null exactly when cost_usd is non-null
}
```

## Tools

Eleven tools are exposed to the model, in this fixed order (`agent/tools.rs::tool_specs`, byte-stable across turns so provider prompt caching keeps hitting). Read-only tools of a turn run in parallel; mutating tools run one at a time, sequentially, with cancellation honoured only *between* them.

| Tool | Kind | Result cap (tokens) |
| --- | --- | --- |
| `search_knowledge` | read | 1,200 |
| `get_knowledge_section` | read | 2,500 |
| `lookup_param` | read | 1,500 |
| `get_extension_config` | read | 800 |
| `list_extensions` | read | 1,200 |
| `list_files` | read | 800 |
| `get_text_file` | read | 1,500 |
| `get_system_info` | read | 300 |
| `set_extension_params` | mutating | 3,000 (backstop) |
| `upload_text_file` | mutating | 3,000 (backstop) |
| `upload_audio_file` | mutating | 3,000 (backstop) |

Every result is capped on a whole-character boundary with a Hebrew truncation note appended; nothing above `MAX_RESULT_TOKENS` (3,000) is ever returned, and each read-only tool has its own tighter cap. A capability that must never be reachable from a prompt is listed in `DENIED_TOOLS` (`file_action`, `delete_extension`, `run_tzintuk`, `run_campaign`, `schedule_campaign`, `send_sms`, `send_fax`, `transfer_units`, `set_password`, `set_customer_details`, `kill_session`, `call_action`) and is refused before dispatch even if a future refactor adds it to the Yemot client.

`set_extension_params` requires a prior `get_extension_config` call on the same path in the same run (`READ_FIRST_REQUIRED` otherwise) and merges only the keys sent — a new extension must also send `type`. `upload_text_file` refuses `ext.ini` paths outright (`EXT_INI_NOT_ALLOWED`; it would replace the whole file instead of merging keys). Both are deferred: unless `auto_apply` is set, the call returns a byte-identical `status=pending_approval` receipt and the real write waits for `approve_actions`.

`upload_audio_file` takes `{ dest_path, attachment_id, reason }` and is **always** in the list, attachments or not, so the serialized tool array stays byte-stable across runs; with no attachments it answers `"אין קבצים מצורפים למשימה"`. `dest_path` must name a `.wav` destination (`DEST_MUST_BE_WAV`) — Yemot's `UploadFile` is called with `convertAudio=1`, which converts any popular source format but requires the path to already carry the converted `.wav` name. Existence of the destination is checked with `list_files` and fails closed (`READ_FAILED`) rather than guessing. The apply reads the file from disk at approval time (never at proposal time) and POSTs one `multipart/form-data` request to `UploadFile` with `token`, `path`, `convertAudio=1` and the file part `qqfile`.

## Approval and undo lifecycle

- A finished run's `RunHandle` **stays in the registry** after `agent:finished` — approval happens after the run ends, and it still needs the proposed actions and the authenticated Yemot client. The handle is evicted only when the *next* `start_agent_run` claims a slot; a second concurrent run is rejected while the current one is unfinished.
- `cancel_agent_run` cancels between tool calls and immediately force-marks the run finished (freeing the slot for the next run), but never interrupts a write already in flight.
- Each proposed action can be applied **once**: `approve_actions` claims the action ids before writing, so a duplicate click (or two overlapping approval calls) returns `"כבר בוצע"` for the ids already claimed instead of writing twice.
- Stale-state check: every proposal carries the `snapshot_hash` the source file had when it was read. Before writing, `approve_actions` re-reads the file; a mismatch refuses the write with `"הקובץ השתנה בשרת מאז ההצעה, הרץ שוב"`, and a failed re-read (state unknown) refuses closed with `"לא ניתן לאמת את מצב הקובץ בשרת, נסה שוב"`.
- After a successful write, every *other* still-pending proposal on the same path is re-stamped with the fresh hash, so approving one action at a time does not make the next one look stale.
- Undo (`undo_action`) restores the previous values for a `set_extension_params` action, or the previous file contents for `upload_text_file`. It re-checks a `post_hash` (the state right after the apply) first and refuses with `"הקובץ השתנה מאז הביצוע, לא ניתן לבטל אוטומטית"` if something else changed it since. Yemot's API has no way to delete a key or a file, so a key that did not exist before the write is restored as empty, and a file that did not exist is left in place.
- An `upload_audio_file` action is applied **without** an undo record (`undo: null`): Yemot exposes no file delete (`file_action` is denied) and the audio it replaced is not recoverable, so the proposal warns about that up front instead of offering an undo that would not work.
- Write state (claimed ids + undo records + the change log, one `RunWriteState` per run) is bounded to 50 runs / 1 hour, whichever is hit first, and is cleared entirely on logout so no live Yemot client outlives the session.
- No context-collapse mechanism exists — history is never rewritten, because that would invalidate the cached prompt prefix. The only lever is `CONTEXT_HARD_LIMIT` (150,000 estimated tokens): past it the run stops with `stop: "truncated"` instead of paying for an ever-growing uncached prompt.
- `RUN_BUDGET` (600s) bounds the whole run, retries included: a computed backoff that would sleep past the remaining budget is not slept — the run fails immediately with the last error instead.
- A repeated identical tool call (same name + canonicalized `path`) is blocked on its third occurrence (`LOOP_DETECTED`); five blocked calls in one run stop it with an `internal` error instead of letting the model spin.

## Knowledge retrieval (`knowledge.rs`)

The Hebrew documentation corpus is compiled into a BM25 lexical index at build time (`build.rs` → `$OUT_DIR/knowledge_index.bin`, loaded via `include_bytes!`). `search_knowledge` returns short snippets (title + a few lines around the match) per hit, except when one result dominates (score ≥ 1.8× the second) — then it returns that hit's full section instead, saving a follow-up `get_knowledge_section` round-trip. A query token matching a system-message code (`m####`) is looked up directly against the message table rather than ranked. The system-messages file (`רשימת הודעות מערכת.txt`) is excluded from ordinary BM25 ranking — it is reachable only through `get_knowledge_section` or a direct message-code query — so it cannot crowd out topical results. `param_catalog()` and `type_catalog()` build the second system-prompt block (the documented key/type index the model must check before writing).

## Settings persisted by the frontend (localStorage keys already in use plus)

- `autoApply` (bool, default false)
- `includeTree` (bool, default true)

## Frontend behaviour

1. On submit in direct mode: call `start_agent_run`, store `run_id`, subscribe to events filtered by `run_id`, show a step list (turn header, tool rows with spinner/✓/✗ + summary, assistant text).
2. On `agent:actions_proposed`: populate the existing approval list (`parsedActions` shape: one row per action with checkbox; expandable diff rows). "בצע" calls `approve_actions`.
3. On `agent:finished`: hide spinner, show a cost line `עלות: $x · מטמון: y% · n סבבים · t שנ׳` (omit cost when null).
4. On `agent:error` with `session_expired`: open the existing MFA/login modal; the user re-runs.
5. Cancel button calls `cancel_agent_run`.
6. Each row of `agent:action_applied` with `ok: true` and a non-null `undo` gets an undo control that calls `undo_action(runId, actionId)`.
