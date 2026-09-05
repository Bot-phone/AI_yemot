# AI yemot

An AI assistant that updates extensions ("שלוחות") in **Yemot HaMashiach**
phone systems. You describe the change in Hebrew; an agent loop reads the
system's current configuration, plans the edits against an embedded corpus of
the official Hebrew documentation, and applies them only after you approve a
before/after diff.

Downloads: [latest release](https://github.com/Bot-phone/AI_yemot/releases/latest).

## Repository layout

| Path | What it is |
| --- | --- |
| `desktop/` | **The product.** Tauri 2 + Svelte 5 desktop app. See [`desktop/README.md`](desktop/README.md). |
| `knowledge/` | The single source of the Hebrew documentation corpus. Embedded into the desktop binary at build time by `desktop/src-tauri/build.rs`. |
| `scraper/` | Python sync job that refreshes `knowledge/` from the Yemot forum. |
| `gas/` | **Frozen legacy.** The original Google Apps Script bot. |
| `index.html` | Static landing page (published via GitHub Pages). |
| `.github/workflows/` | `desktop.yml` (tests + releases), `sync-yemot-docs.yml` (corpus sync), `deploy-gas.yml` (manual only). |

## Status

- The **desktop app is the primary and only actively developed client.** All new
  features go there.
- The **Google Apps Script bot in `gas/` is frozen legacy** ("script mode"). It
  is kept for existing users; `deploy-gas.yml` is `workflow_dispatch` only, so
  nothing is deployed automatically. Security and critical-breakage fixes only.
- The old browser client that lived in `index.html` — which sent the Yemot token
  as a URL query parameter to Apps Script — has been removed and replaced by a
  static landing page.

## Development

```bash
cd desktop && npm ci
npm run tauri dev            # run the app
npm run check                # frontend
cd src-tauri && cargo test --lib   # backend
```

Conventions for contributors and coding agents are in [`CLAUDE.md`](CLAUDE.md).
