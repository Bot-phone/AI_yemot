# AI yemot

**An AI-based editor for Yemot HaMashiach phone lines** — עורך קווי ימות המשיח
מבוסס AI. Not a chat: a desktop workspace where you open your line, describe a
task in Hebrew, and get concrete `ext.ini` and file changes back as a reviewable
diff.

### Who it is for

Anyone who maintains a Yemot HaMashiach line — gabbaim, organisations, IVR
integrators — and wants to change extension behaviour without hand-editing
`ext.ini` keys or hunting through the forum documentation.

### The workflow

1. **Connect** (התחברות) — system number, password, MFA if the line requires it.
   The Yemot token stays on your machine, in the OS credential store.
2. **Browse the line structure** (מבנה הקו) — the live extension tree; open a
   שלוחה to inspect its keys, its files and its raw `ext.ini`.
3. **Describe a task** (משימה) — plain Hebrew, e.g. "בשלוחה 3 תעשה תפריט עם
   שלוש אפשרויות". Attach audio files if the task needs recordings.
4. **Review the proposed changes** (שינויים מוצעים) — every mutating step is a
   proposal with a before/after diff, a risk marker and warnings. Nothing is
   written yet.
5. **Approve** (בצע) — pick the proposals you want; only those are written. The
   editor re-reads the server first and refuses if the file moved since the
   proposal was made.
6. **Undo** (ביטול) — the change log (יומן שינויים) keeps every applied change
   and can reverse it while the server state has not moved.

Not satisfied? Refine the same task ("דייק את המשימה") instead of starting over
— the editor continues from the same task with the status of the previous
proposals, it does not open a new conversation.

### Features

- Live line structure browser and an extension inspector (keys, files, raw
  `ext.ini`).
- Task → proposed changes → approval → change log, with per-action undo.
- Audio attachments: attach `wav` / `mp3` / `m4a` / `ogg` / `wma` / `aac` files
  to a task and let the editor place them on the line.
- The official Hebrew Yemot documentation is compiled into the binary and
  searched locally — no forum tab, no runtime download.
- Direct API access: the Yemot token never leaves your machine except to call
  `call2all.co.il`.
- A per-task cost estimate and token/cache counters.
- Built-in update check against GitHub releases.

### Installing

Download the installer for your OS from the
[latest release](https://github.com/Bot-phone/AI_yemot/releases/latest) —
Windows (`.msi` / `.exe`, or the single-file `-portable.exe` that needs no
installation), macOS (`.dmg`), Linux (`.AppImage` / `.deb`).

The installers are **not code-signed**, so Windows SmartScreen shows a
"Windows protected your PC" warning on first run: choose *More info* → *Run
anyway*. macOS Gatekeeper needs the equivalent *Open anyway* in
System Settings → Privacy & Security.

### AI provider (BYOK)

The editor ships with no AI key. You bring your own — Claude, Gemini, OpenAI,
Groq, or any OpenAI-compatible endpoint — and the key is stored in the OS
credential store, never in a config file and never sent anywhere but that
provider. The cost line shown after each task is an **estimate** computed from
public list prices as of a fixed date; free tiers, promotions and provider price
changes are not visible to the app, and an unknown model shows no cost at all.

## Use with care

This editor **writes to a live phone system**. A wrong change can silence a
line, misroute callers or overwrite recordings. Review every proposed change
before approving it, keep automatic apply off unless you know exactly what a
task will do, and use the change log to undo promptly. The software is provided
as is, **without any warranty and without liability for damage**, including
changes made to a phone system; see [`LICENSE`](LICENSE).

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

## License

Source-available, not open source. See [`LICENSE`](LICENSE) (English and
Hebrew). In short:

- **Source code** — personal, non-public, non-commercial use only; keep the
  attribution to this project.
- **Built software** (the installers on the Releases page) — may be shared
  publicly, free of charge, for non-commercial use, with attribution.
- The Yemot HaMashiach documentation in `knowledge/` belongs to its authors.

Any other use needs written permission from the copyright holder.
