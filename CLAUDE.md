# Repo conventions

An AI-based **editor for Yemot HaMashiach phone lines** — not a chat and not an
"assistant". The user browses the line structure (מבנה הקו), inspects an
extension ("שלוחה"), describes a task (משימה) in Hebrew, reviews the proposed
changes (שינויים מוצעים) as a diff, approves, and can undo from the change log
(יומן שינויים). The model is the engine that turns a task into concrete
`ext.ini` / file changes; user-facing text uses editor vocabulary ("משימה",
"הצע שינויים", "בצע", "דייק את המשימה"), never chat vocabulary. Read
`README.md` for the layout and `desktop/README.md` for the desktop architecture
before making changes.

## Where things live

- `desktop/` — the product. Tauri 2 backend in `desktop/src-tauri/src/`, Svelte 5
  frontend in `desktop/src/`. New features go here.
  - `src-tauri/src/lib.rs` — the single registry of Tauri commands.
  - `src-tauri/src/agent/` — the agent loop (`runner.rs`), prompt (`prompt.rs`),
    tools (`tools.rs`), LLM providers (`providers/`).
  - `src-tauri/src/yemot.rs`, `yemot_ini.rs` — Yemot API and `ext.ini` format.
  - `src-tauri/src/knowledge.rs`, `knowledge_text.rs`, `build.rs` — the index.
- `knowledge/` — the documentation corpus. See the hard rules below.
- `scraper/` — Python job that refreshes `knowledge/`.
- `gas/` — frozen legacy Google Apps Script bot. Do not add features; security
  and critical fixes only. Its workflow is `workflow_dispatch` only.
- `index.html` — static landing page. No token handling, no external CDNs.

## Commands

```bash
cd desktop/src-tauri && cargo test --lib   # Rust tests — must pass
cd desktop/src-tauri && cargo check        # type check
cd desktop && npm run check                # svelte-check — must pass
cd desktop && npm run build                # frontend build
cd desktop && npm run tauri dev            # run the app
```

CI (`.github/workflows/desktop.yml`) runs exactly these on PRs and pushes to
`main`; a `v*` tag builds installers and publishes a GitHub release.

## Hard rules

1. **No AI attribution in commits.** Never add a `Co-Authored-By` trailer, a
   "Generated with…" line, or any mention of Claude/AI in a commit message, PR
   body, or code comment.
2. **The knowledge corpus lives only in `./knowledge`.** Do not duplicate,
   vendor, or copy it into `desktop/`. `desktop/src-tauri/build.rs` embeds it at
   build time (BM25 index → `$OUT_DIR/knowledge_index.bin` →
   `include_bytes!` in `knowledge.rs`). Editing the corpus requires a rebuild.
   Tokenization/chunking rules live in `src/knowledge_text.rs`, which is
   included by both `build.rs` and `knowledge.rs` — never fork that logic.
3. **The system prompt must stay byte-stable.** `agent/prompt.rs` is built to
   emit identical bytes across runs so provider prompt caching keeps hitting.
   Do not introduce timestamps, HashMap iteration order, locale-dependent
   formatting, or any run-varying content into the prompt. Changing prompt text
   is a deliberate, reviewed act — not a drive-by edit.
4. **The version of record is `desktop/src-tauri/Cargo.toml`.**
   `tauri.conf.json` has no `version` key on purpose, and `updater.rs` compares
   `CARGO_PKG_VERSION` against the latest GitHub release tag. Bump Cargo.toml
   and tag `v<version>` to release.
5. **Mutating actions require approval.** Anything that writes to a phone system
   goes through the approve/diff path in `runner.rs`; never apply writes
   silently.
6. **Secrets fail closed.** No hard-coded fallback secrets. If a secret is not
   configured, reject or skip the operation and log why.
