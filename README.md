<h1 align="center">TokenTrail</h1>

<p align="center">
  One dashboard for all the AI tokens you burn while coding.
</p>

<p align="center">
  <img alt="License" src="https://img.shields.io/badge/license-MIT-black?style=flat-square">
  <img alt="Platform" src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-black?style=flat-square">
  <img alt="Built with" src="https://img.shields.io/badge/built%20with-Tauri%20%2B%20Svelte-black?style=flat-square">
</p>

---

TokenTrail is a desktop app that reads the usage logs your AI coding tools already write on your machine and turns them into a single, clear picture — tokens, cost estimates, sessions, projects, and daily trends. No accounts, no setup: open it and your history is there.

## Supported tools

| Harness | Notes |
| --- | --- |
| ZCode | Local session logs + subagents |
| Claude Code | Includes subagent sessions |
| Codex | Sessions + subagent threads |
| OpenCode | Per-project usage |
| Gemini CLI | Session history |
| Antigravity | Conversation databases |

If a tool isn't installed, it's simply skipped — TokenTrail shows whatever it finds.

## What you get

- **Overview** — totals, sessions, active-day streak, cache hit rate, and spend estimate at a glance
- **Models & Families** — usage and cost broken down per model or model family
- **Trends** — how your usage moves day to day
- **Projects** — which projects consume the most tokens
- **Activity** — a live feed of recent sessions as they sync in
- **Settings** — merge duplicate model names, hide noise, export your data

## Private by design

- Everything runs **locally** — no accounts, no telemetry, no cloud.
- Your harness files and databases are opened **read-only**. TokenTrail never writes to them.
- Your history lives in TokenTrail's own local database and can be exported anytime.

> **A note on cost:** figures are API-equivalent estimates based on bundled list prices — not what you actually pay on a subscription.

## Getting started

Download the latest build for your platform from the [Releases](../../releases) page, launch it, and that's it. TokenTrail syncs in the background and refreshes automatically.

### Building from source

You'll need [Bun](https://bun.sh) and [Rust](https://rustup.rs).

```bash
bun install
bun run tauri dev    # run in dev mode
bun run tauri build  # produce a production build
```

## Tech stack

Tauri 2 (Rust backend) · SvelteKit + Svelte 5 frontend · ECharts · SQLite

## License

[MIT](LICENSE) — do whatever you want with it.
