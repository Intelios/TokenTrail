# TokenTrail — Workspace Instructions

TokenTrail is a Tauri desktop app that aggregates AI-coding usage across local harnesses (ZCode, Claude Code, Codex, OpenCode, Gemini CLI) and shows it in a SvelteKit + ECharts dashboard.

## Repository layout

- `src/` — SvelteKit frontend (TypeScript, Svelte 5 runes, `echarts`).
- `src-tauri/` — Rust backend.
  - `src-tauri/src/collectors/` — one file per harness that reads harness-owned files/SQLite read-only and normalizes rows into `UsageEvent`s.
  - `src-tauri/src/store.rs` — TokenTrail's own SQLite (`usage.db`) in `app_data_dir`.
  - `src-tauri/src/aggregate.rs` — queries that power the UI.
  - `src-tauri/src/commands.rs` — Tauri command handlers.
  - `src-tauri/src/pricing.rs` — bundled list-price lookup table from `pricing/pricing.json`.
  - `src-tauri/src/models.rs` — `UsageEvent`, `Source` enum, shared types.
  - `src-tauri/fixtures/` — sample JSONL files used by tests.
- `static/` — favicon and logos.

## Build / dev / check commands

Use `bun` for the frontend:

```bash
bun install
bun run dev        # Vite dev server on localhost:1420
bun run build      # static SPA into build/
bun run check      # svelte-check + svelte-kit sync
```

Use `cargo` for the Tauri backend:

```bash
cd src-tauri
cargo build
cargo test
cargo clippy
```

The Tauri CLI is driven via `bun run tauri -- <cmd>`:

```bash
bun run tauri dev    # build frontend + launch desktop app
bun run tauri build  # production build
```

## Architecture rules

- **Frontend is a static SPA.** `adapter-static` with `fallback: "index.html"`; `src/routes/+layout.ts` sets `ssr = false`. Do not introduce server-side rendering or server routes.
- **Frontend never reads harness files directly.** All data comes from Tauri commands exposed through `src/lib/api.ts`. Add new commands in `commands.rs` and immediately expose them in `api.ts`.
- **Backend never writes to harness files/databases.** All collectors open external databases read-only (`store::open_readonly`) and read log files with `read_tail`. TokenTrail's own database is the only writable store.
- **Event identity is (source, source_event_id).** New collectors must provide stable IDs so re-ingestion is idempotent; the store upserts on conflict.
- **Model strings are normalized before pricing lookup.** Use the same logic in both Rust (`pricing::normalize_model`) and TypeScript (`format.ts` source names). Do not duplicate pricing math on the frontend.
- **Thread / subagent handling matters.** Claude Code flags `subagents/` paths and `isSidechain`; Codex uses `thread_source == "subagent"`. Keep subagent detection with the collector that knows the harness schema.

## Coding conventions

- TypeScript: strict, `type: "module"`, Svelte 5 runes (`$state`, `$derived`, `$effect`, `$props`).
- Rust: `?`/`.map_err(|e| format!("...: {e}"))` for command errors; `Result<..., String>` to Tauri.
- Rust formatting is in i64/i64 optionals; reasoning tokens are never included in "total tokens" — see `aggregate.rs` constant `TOKENS`.
- Frontend styling uses the dark theme in `src/app.css`. Shared color palette / tokens are in CSS variables; do not introduce new CSS-in-JS libraries.
- Imports use SvelteKit aliases: `$lib/...` for shared code. No custom path aliases beyond that.

## Communication between layers

- Rust emits `sync-done` after each background sync via `app_handle.emit("sync-done", &stats)`.
- `+layout.svelte` listens via `@tauri-apps/api/event` and dispatches `window.dispatchEvent(new CustomEvent('tt-sync'))`.
- Every route that needs live data listens for `tt-sync` and calls `load()` again.
- Do not add a global Svelte store unless you are adding cross-route state; the event-based refresh is the current pattern.

## Data / schema gotchas

- Timestamps are stored as **epoch milliseconds** in `usage_event.ts`.
- OpenCode stores seconds in some builds; `opencode.rs` multiplies values `< 100_000_000_000` by 1000.
- `aggregate.rs` computes "active days" and streaks in UTC; the current streak intentionally tolerates "today hasn't happened yet".
- Model display names resolve through the `model_alias` table (alias → canonical) at query time in `aggregate.rs`; raw `usage_event.model` is never rewritten. Merge/unmerge commands live in `store.rs` (`merge_models`, `remove_aliases_for`).
- User-hidden models live in the `hidden_model` table (display names); every aggregate query in `aggregate.rs` excludes them via the `NOT_HIDDEN` fragment. Hiding never touches event data; `export_data` is unfiltered.
- Cost is an **API-equivalent estimate** from bundled list prices, not actual subscription spend. Mention this whenever UI text touches cost.
- `pricing.json` lists exact models and prefix-ordered families. If you add a new model family, add an entry with a longer-specific prefix *before* catch-all prefixes.

## Tauri configuration

- `src-tauri/tauri.conf.json`: devUrl `http://localhost:1420`, frontendDist `../build`.
- Default capability is `core:default` + `opener:default`; any new plugin permission must be added there.
- The app window default size is 1240×820 with a 940×640 minimum.

## Tests

- Rust tests live in `#[cfg(test)]` blocks inside each collector/aggregate/pricing file.
- Run focused tests per module: `cargo test codex`, `cargo test zcode`, etc.
- Frontend has no test runner configured yet; verify with `bun run check` and manual `bun run dev` + `bun run tauri dev`.

## Files to read before changing sensitive areas

- New harness source → read an existing collector (`claude_code.rs` is the most complete JSONL example) and `models.rs`.
- New aggregate metric → read `aggregate.rs` and mirror types in both `commands.rs` and `src/lib/api.ts`.
- New UI route/page → read `src/routes/+layout.svelte` and `src/app.css` first.
- Pricing model support → read `src-tauri/src/pricing.rs` and `src-tauri/pricing/pricing.json`.
