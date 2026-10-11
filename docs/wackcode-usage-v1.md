# WackCode usage ledger v1

WackCode owns `~/Library/Application Support/com.wackcode.desktop/usage/v1/YYYY-MM/<writer-uuid>.jsonl`.
TokenTrail reads complete newline-terminated records only, incrementally and read-only.
Files are retained indefinitely, including after chat deletion. Recording is enabled by
default and can be disabled in WackCode Settings > Integrations. No older chats are exported.

Each JSON object contains `v: 1`, UUID `id`, completion timestamp `ts` (epoch milliseconds),
`duration_ms`, `session_id` (owning WackCode chat), nullable `project` (original project root),
nullable `workspace` (execution directory), `provider`, `model`, `purpose`, nullable
`subagent_id`, `outcome`, optional `mode`, and nullable `tokens`.

Purposes: `chat`, `subagent`, `title`, `compaction`, `branch_summary`, `goal_verification`,
`commit_message`. Outcomes: `completed`, `failed`, `cancelled`. Sub-agent identity is
independent of purpose: a child's compaction still belongs to that child and parent chat.

`mode` is the app mode the call ran in: `code` (the coding agent) or `chat` (Chat mode).
It is additive and optional; records written before Chat mode shipped carry no `mode`, and
readers treat an absent `mode` as `code`. A WackCode chat belongs to one mode for life, so
every record of a session shares its `mode`. Mode is orthogonal to purpose: `purpose: chat`
with `mode: chat` is a Chat mode turn, while `purpose: chat` with `mode: code` (or absent)
is a Code mode turn.

`tokens` contains nonnegative integers `input`, `output`, `cache_read`, `cache_write`.
Input excludes both cache categories; output includes reasoning. These four categories
are additive. Null means the provider exposed no usable counts, never estimated usage.
TokenTrail excludes null counts from measured aggregates and calculates API-equivalent
estimated cost using its own pricing table. WackCode's cost fields are not exported.

Identity is `(wackcode, id)`. Retries of ledger delivery preserve the ID; distinct model
invocations get distinct IDs. Readers must deduplicate and tolerate interrupted tails,
replacement files and additive fields (`provider_name` and `mode` were added this way).
Unsupported versions must be diagnosed, not guessed.
No prompts, responses, titles, tool arguments, URLs, credentials or error bodies are exported.

Coverage is WackCode-managed model calls. Provider-internal attempts without exposed usage,
extension-owned networking and completion data lost to a crash cannot be reconstructed.
