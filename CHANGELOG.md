# Changelog

All notable changes to AI yemot. Versions follow the crate version in
`desktop/src-tauri/Cargo.toml`, which is also the release tag (`v<version>`).

## 1.0.0

The first release of AI yemot as a **desktop editor for Yemot HaMashiach phone
lines**. The browser client that posted the Yemot token to a Google Apps Script
is gone; the Apps Script bot is frozen legacy, and `index.html` is now only a
static landing page.

### Line editor workspace

- A Tauri 2 + Svelte 5 desktop app: connect to a line with system number,
  password and MFA, browse the line structure, describe a task in Hebrew, review
  the proposed changes, approve, undo.
- Proposed changes are shown per extension with a before/after diff, a risk
  marker (`low` / `overwrite` / `destructive`) and warnings for undocumented
  keys; the approval list is checkbox-per-change, so partial approval is normal.
- The official Hebrew Yemot documentation is compiled into the binary as a BM25
  index and searched locally — no forum tab, no runtime download. Rich markdown
  rendering with internal cross-file links, sanitized and served under the app
  CSP.
- Hebrew-first UI with an English locale and correct RTL/LTR direction handling.
- Bring your own AI key: Claude, Gemini, OpenAI, Groq or any OpenAI-compatible
  endpoint, with an explicit model id when you want one. Each task ends with an
  estimated cost plus token and cache-hit counters.

### Agent loop safety

- Nothing is written to a line without approval. Before applying, the editor
  re-reads the file and refuses if it moved since the proposal; a failed re-read
  also refuses, rather than writing blind.
- Each proposal applies at most once, approvals run sequentially, and every
  applied change keeps an undo record that is itself re-checked before reversal.
- Destructive Yemot capabilities (deleting extensions, campaigns, SMS/fax,
  transferring units, password and customer changes) are on a deny list and are
  refused before dispatch, whatever the model asks for.
- Guards against runaway tasks: a per-run time budget, a hard context limit that
  stops the task instead of paying for an uncached prompt, repeated-tool-call
  loop detection, and a run lifecycle that survives a panicking turn.
- Writes are validated against a per-type `ext.ini` key catalog built from the
  corpus, so an undocumented key is flagged rather than silently written.

### Streaming

- Every provider streams the model's answer to the UI over SSE; the text appears
  as it is produced and is replaced by the authoritative block when the turn
  closes, so a dropped chunk cannot corrupt what you read.
- Thinking / thought summaries are never streamed. A retry cleanly discards the
  abandoned partial block, and an endpoint that rejects streaming is retried
  once without it.

### Secrets and keychain

- The Yemot token and all provider keys live in the OS credential store
  (Windows Credential Manager / macOS Keychain / Secret Service), never in
  `localStorage` and never in a config file, behind a fixed name allowlist.
- The Yemot token never leaves the Rust side except to call `call2all.co.il`,
  and write state is cleared on logout so no authenticated client outlives the
  session. Missing secrets fail closed.
- Hardened CSP, minimal opener scope, and error paths that cannot echo a key.

### CI and release

- `desktop.yml` runs Rust tests, clippy (`-D warnings`), the frontend check and
  the frontend build on every PR and push to `main`; a `v*` tag builds Windows,
  macOS and Linux installers and publishes the GitHub release the in-app updater
  reads.
- All GitHub Actions are pinned to commit SHAs with least-privilege permissions;
  the Apps Script deploy workflow is `workflow_dispatch` only.
- Installers are not code-signed, so Windows SmartScreen warns on first run.
