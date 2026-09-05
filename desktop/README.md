# AI yemot — desktop app

The product: a Tauri 2 desktop app (Rust backend + Svelte 5 / SvelteKit
frontend) that is an **AI-based editor for Yemot HaMashiach phone lines**. The
user browses the line structure, describes a task, reviews proposed changes as a
diff, approves, and can undo. An LLM agent loop is the engine behind the
proposals — it is not a chat surface.

## Requirements

- Rust stable (with the platform toolchain Tauri 2 needs)
- Node.js 20+
- Linux only: `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf`

## Commands

Run from `desktop/` unless noted.

| What | Command |
| --- | --- |
| Install frontend deps | `npm ci` |
| Dev app (Vite + Tauri) | `npm run tauri dev` |
| Frontend check (svelte-check) | `npm run check` |
| Frontend build only | `npm run build` |
| Rust tests | `cargo test --lib` (in `src-tauri/`) |
| Rust type check | `cargo check` (in `src-tauri/`) |
| Rust lint | `cargo clippy --lib -- -D warnings` (in `src-tauri/`) |
| Production bundle | `npm run tauri build` |

CI runs the test, lint, check and build steps above (not `cargo check`, which the
lint step subsumes) — see `.github/workflows/desktop.yml`.

## Architecture

```
Svelte UI — the line editor workspace (src/routes/+page.svelte, src/lib/components/)
  line structure tree · extension inspector · task box (+ audio attachments)
  proposed-changes list with diffs · change log with undo
        │  invoke(...)  — Tauri commands, registered in src-tauri/src/lib.rs
        ▼
Rust backend (src-tauri/src/)
  agent/runner.rs   the run: task → tool calls → proposals → approval → apply
  agent/prompt.rs   the system prompt (byte-stable, see CLAUDE.md)
  agent/tools.rs    tool definitions; read-only tools of a turn run in parallel
  agent/pricing.rs  list-price table + PRICE_LIST_DATE for the cost estimate
  agent/providers/  anthropic.rs / gemini.rs / openai.rs behind one trait
  knowledge.rs      BM25 query side over the embedded index
  yemot.rs          Yemot API client (login, MFA, read/write extensions, uploads)
  yemot_ini.rs      parsing/emitting the ext.ini extension format
  secrets.rs        OS keychain storage for the Yemot token and provider keys
  updater.rs        GitHub releases/latest check (see below)
        ▼
Yemot HaMashiach API (call2all.co.il/ym/api)
```

### Line structure and inspector

`get_extension_tree(token, root, depth)` builds the nested extension tree
(מבנה הקו) from `YemotClient::list_extensions`; depth is clamped to 1..=4 and an
empty root means the whole line. `read_extension(token, path)` returns one
extension's parsed keys in file order, its file list and its raw `ext.ini`
(capped at 64 KiB). Both are strictly read-only — they never touch the write
state — and both surface a `session_expired` error the same way runs do, so the
UI can reuse its MFA/login modal.

### Tasks and continuation

A task is one run. Refining it ("דייק את המשימה") calls `continue_agent_run`
with the parent run id: the new run starts from the parent's full transcript —
so the cached prompt prefix still hits — plus a short status block listing each
previous proposal as בוצע / בוטל / לא אושר, followed by the refinement. It emits
the same events and gets a new `run_id`. This is a refinement of the same task,
not a chat turn: the parent must be the run in the registry and finished, and a
transcript already past `CONTEXT_HARD_LIMIT` is refused rather than continued.

### Audio attachments

Files attached to a task (via the Tauri dialog plugin — `tauri-plugin-dialog`
plus the `dialog:allow-open` capability in `src-tauri/capabilities/default.json`)
are listed to the model as ids only, in a `[קבצים מצורפים]` block on the first
user message. The `upload_audio_file` tool resolves `attachment_id` to a local
path from the run's own list, never from model input; non-audio types and files
over 25 MiB are rejected at run start. It is a mutating tool, so it produces a
proposal with a diff like any other write, marked `overwrite` when the target
file already exists — and an applied upload has no automatic undo.

### Change log

`list_applied_changes()` reads the applied-write records (run id, action id,
kind, path, timestamp, whether an undo is still available and whether it was
already undone), newest first. This is what backs the UI's יומן שינויים, so the
list survives a page reload instead of living only in component state.

### Cost estimate

After each task the UI shows an estimated cost alongside token and cache
counters. The numbers come from `agent/pricing.rs`, a table of **public list
prices** frozen at `PRICE_LIST_DATE`; the app cannot see free tiers, promotional
rates or provider price changes, and a model missing from the table yields no
cost at all rather than a guess. `agent:finished` carries `price_list_date`
exactly when it carries a `cost_usd`.

### Knowledge corpus

The Hebrew documentation corpus lives **only** in the repo-root `knowledge/`
directory. `src-tauri/build.rs` builds a BM25 lexical index over it at build
time and writes a compact varint binary to `$OUT_DIR/knowledge_index.bin`, which
`knowledge.rs` pulls in with `include_bytes!`. There is no runtime download and
no extra crate: tokenization/chunking rules live in `src/knowledge_text.rs`,
which is `#[path]`-included by both `build.rs` and `knowledge.rs` so the index
and the queries can never drift apart.

Changing `knowledge/` therefore requires a rebuild, not a restart.

### Approvals

The editor never mutates a line silently: mutating tool calls become *proposed
changes* (שינויים מוצעים), surfaced in the UI with a before/after diff, and are
applied only after `approve_actions` comes back from the frontend. Applied
actions land in the change log and can be reversed
with `undo_action` as long as the server state has not moved since. Full
lifecycle details (stale-state checks, double-approval, undo) are in
[`docs/agent-contract.md`](docs/agent-contract.md).

### Updates

`updater.rs` reads `https://api.github.com/repos/Bot-phone/AI_yemot/releases/latest`
and compares `tag_name` against `CARGO_PKG_VERSION`. The version of record is
`src-tauri/Cargo.toml`; `tauri.conf.json` intentionally has no `version` key so
it inherits from Cargo. Releases are produced by the `release` job on `v*` tags.
