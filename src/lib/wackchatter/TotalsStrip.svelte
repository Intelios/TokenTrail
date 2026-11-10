<script lang="ts">
  import AnimatedNumber from "$lib/AnimatedNumber.svelte";
  import Spark from "$lib/Spark.svelte";
  import type { Stats } from "./types";
  import { fmtTokens } from "$lib/format";
  import { foldActivity } from "./buckets";
  import { count } from "./labels";

  let { stats }: { stats: Stats } = $props();

  /** The last 12 local months of a series, for the strip's sparklines. */
  const recent = (rows: readonly (readonly [string, number, number])[]) =>
    (foldActivity(rows.map((r) => ["all", r[1], r[2]] as const), "month").series.get("all") ?? []).slice(-12);

  const sparks = $derived({
    messages: recent(stats.hours.map(([t, n]) => ["", t, n] as const)),
    replies: recent(stats.cast_activity),
    swipes: recent(stats.model_activity),
  });
</script>

<div class="strip">
  <div class="cell">
    <span>Chats</span><strong><AnimatedNumber value={stats.totals.chats} format={count} /></strong>
  </div>
  <div class="cell">
    <span>Messages</span><strong><AnimatedNumber value={stats.totals.messages} format={count} /></strong>
    <Spark values={sparks.messages} width={110} color="var(--ink)" />
  </div>
  <div class="cell">
    <span>Replies</span><strong><AnimatedNumber value={stats.totals.replies} format={count} /></strong>
    <Spark values={sparks.replies} width={110} color="var(--ink)" delay={80} />
  </div>
  <div class="cell">
    <span>Generated swipes</span><strong><AnimatedNumber value={stats.totals.swipes} format={count} /></strong>
    <Spark values={sparks.swipes} width={110} color="var(--ink)" delay={160} />
  </div>
  <div class="cell lit">
    <span>Time in scene · estimated</span><strong
      >{Math.floor(stats.totals.active_minutes / 60)}<small>h</small>
      {stats.totals.active_minutes % 60}<small>m</small></strong
    >
  </div>
  <div class="cell">
    <span>Measured completion tokens</span><strong>{fmtTokens(stats.totals.measured_tokens)}</strong>
  </div>
  <div class="cell est">
    <span>Estimated completion tokens</span><strong>{fmtTokens(stats.totals.estimated_tokens)}</strong>
    <small class="tag">Never priced</small>
  </div>
</div>
<p class="wc-note">
  Completion counts from saved replies · Sparklines are the last 12 months · Full generation
  accounting lives in Usage
</p>

<style>
  .strip {
    display: grid;
    grid-template-columns: repeat(7, minmax(0, 1fr));
    border: 2px solid var(--ink);
    gap: 2px;
    background: var(--ink);
  }
  .cell {
    background: var(--bone);
    padding: 15px 13px 12px;
    display: flex;
    flex-direction: column;
    gap: 11px;
    min-width: 0;
    overflow: hidden;
  }
  .cell > span {
    font: 9.5px/1.4 var(--font-mono);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    min-height: 26px;
  }
  .cell strong {
    font: 32px/1 var(--font-disp);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .cell strong small {
    font-size: 17px;
  }
  .cell.lit {
    background: var(--aura);
  }
  .cell.est strong {
    color: var(--vio);
  }
  .tag {
    font: 9px var(--font-mono);
    text-transform: uppercase;
    letter-spacing: 1px;
    border: 1px solid currentColor;
    padding: 2px 5px;
    align-self: flex-start;
    color: var(--vio);
  }
  @media (max-width: 1100px) {
    .cell {
      padding: 13px 9px 10px;
    }
    .cell strong {
      font-size: 27px;
    }
  }
</style>
