# AI yemot — desktop app

The primary client: a Tauri 2 desktop app (Rust backend + Svelte 5 / SvelteKit
frontend) that updates extensions ("שלוחות") in Yemot HaMashiach phone systems
through an LLM agent loop.

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
| Production bundle | `npm run tauri build` |

CI runs exactly the check/test set above — see `.github/workflows/desktop.yml`.

## Architecture

```
Svelte UI (src/routes/+page.svelte, src/lib/)
        │  invoke(...)  — Tauri commands, registered in src-tauri/src/lib.rs
        ▼
Rust backend (src-tauri/src/)
  agent/runner.rs   the agent loop: plan → tool calls → approval → apply
  agent/prompt.rs   the system prompt (byte-stable, see CLAUDE.md)
  agent/tools.rs    tool definitions; read-only tools of a turn run in parallel
  agent/providers/  anthropic.rs / gemini.rs / openai.rs behind one trait
  knowledge.rs      BM25 query side over the embedded index
  yemot.rs          Yemot API client (login, MFA, read/write extensions)
  yemot_ini.rs      parsing/emitting the ext.ini extension format
  secrets.rs        OS keychain storage for the Yemot token and provider keys
  updater.rs        GitHub releases/latest check (see below)
        ▼
Yemot HaMashiach API (call2all.co.il/ym/api)
```

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

The loop never mutates a phone system silently: mutating tool calls are
collected, surfaced in the UI with a before/after diff, and only applied after
`approve_actions` comes back from the frontend. Applied actions can be reversed
with `undo_action` as long as the server state has not moved since. Full
lifecycle details (stale-state checks, double-approval, undo) are in
[`docs/agent-contract.md`](docs/agent-contract.md).

### Updates

`updater.rs` reads `https://api.github.com/repos/Bot-phone/AI_yemot/releases/latest`
and compares `tag_name` against `CARGO_PKG_VERSION`. The version of record is
`src-tauri/Cargo.toml`; `tauri.conf.json` intentionally has no `version` key so
it inherits from Cargo. Releases are produced by the `release` job on `v*` tags.
