# Mistral Vibe

TokenTrail discovers the terminal `vibe` application's default session root at
`~/.vibe/logs/session`. The collector targets the unified Harness store shipped
with Vibe 3 (verified against the installed 3.0.4). No Vibe command, account API,
credentials, or configuration changes are needed.

## Records and identity

- `unified/<session-id>/meta.json` identifies the session and maps its current
  model alias to the provider model name.
- `generations/*/runtime-state.json` supplies the retained journal's initial
  project directory, model pin, and parent session identity. Metadata updates
  within a journal are `session_relocated` records.
- `journal/*.jsonl` supplies per-request usage. Only version 1 `core_input`
  records whose command is `completion_succeeded` are counted. The accompanying
  `action_result`, projection totals, and transcript chunks repeat the same work
  and are not counted.
- Event identity is `(source=vibe, session_id:action_id)`, independent of journal
  segment names, replay sequence, timestamps, and snapshot generation. Timestamps
  come from `payload.input.determinism.time_unix_ms`.
- Subagents are separate sessions, flagged by `parent_session_id`. Their usage
  is read from their own journal rather than parent tool results.

The Harness reports input including cached input and cache writes. TokenTrail
subtracts both into separate buckets. Output already includes reasoning, so the
reasoning field is preserved without adding it to totals or billing it twice.
Missing input/output, negative or invalid counts, empty IDs, and invalid times
are skipped; no provider counts are estimated. Provider route and timings are
left unknown because these records do not identify them.

Changed sessions are reread as a unit, using per-session fingerprints of the
metadata, snapshot files, and journal segments. This retains context across
segments and handles replacement/truncation. Partial tail lines are retried on
the next sync. Upserts make replays, resumed sessions, and rescans idempotent.
Unreadable sessions surface a sync error while other sessions still import.

## Coverage limits

Vibe prunes superseded generations and retains only two journal segments.
TokenTrail imports all per-call usage still present when it syncs and retains
those events in its own database after Vibe prunes them. Keep TokenTrail running
while using Vibe for the best coverage. Calls pruned before the first sync, or
between syncs, cannot be recovered with per-request dates and model attribution.
Session totals are not fabricated into extra requests.

Older `session_*/meta.json` summary-only logs, session exports, transcript chunks,
plugin caches, and custom session log roots are not imported. A model pin whose
API name is no longer available remains under its recorded alias. Unknown model
prices stay unknown rather than inheriting a generic Mistral rate.

## Pricing

Costs are API-equivalent estimates from bundled list prices, not subscription
spend. Mistral Large 4 (including `mistral-large-4-0`) is priced at $0.70 input,
$0.07 cached input, and $2.10 output per million tokens, per
[Mistral's model documentation](https://docs.mistral.ai/models/mistral-large-4-0).
Devstral and Codestral names group under the Mistral family, but unbundled model
prices are left unknown.

## Validation

Run `cargo test vibe --lib` in `src-tauri` for synthetic regression coverage.
An opt-in smoke test reads local Vibe files into a temporary TokenTrail database:

```sh
VIBE_TEST_HOME="$HOME" cargo test local_vibe_history --lib -- --ignored --nocapture
```

It imports no transcript bodies into TokenTrail and never writes to Vibe's store.
