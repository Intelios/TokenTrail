<script lang="ts">
  import { onMount } from 'svelte';
  import type { EChartsOption } from 'echarts';
  import Chart from '$lib/Chart.svelte';
  import AnimatedNumber from '$lib/AnimatedNumber.svelte';
  import Spark from '$lib/Spark.svelte';
  import { api, type DailyRow, type ModelRow, type Overview } from '$lib/api';
  import {
    fmtCost,
    fmtTokens,
    fmtTokensSplit,
    sourceColor,
    sourceLabel,
    MODEL_PALETTE,
  } from '$lib/format';
  import { TOOLTIP, ANIM, stackedBand } from '$lib/chartTheme';

  // stack order matches the Marathon mockup legend
  const SOURCE_ORDER = ['claude_code', 'codex', 'zcode', 'antigravity', 'opencode', 'gemini'];

  let overview = $state<Overview | null>(null);
  let daily = $state<DailyRow[]>([]);
  let models = $state<ModelRow[]>([]);
  let error = $state('');

  async function load() {
    try {
      const [o, d, m] = await Promise.all([api.overview(), api.daily(150), api.byModel(30)]);
      overview = o;
      daily = d;
      models = m.slice(0, 5);
      error = '';
    } catch (e) {
      error = String(e);
    }
  }

  onMount(() => {
    load();
    const h = () => load();
    window.addEventListener('tt-sync', h);
    return () => window.removeEventListener('tt-sync', h);
  });

  // ── hero sparkline: daily totals across the loaded window ──
  const dailyTotals = $derived.by(() => {
    const byDate = new Map<string, number>();
    for (const r of daily) byDate.set(r.date, (byDate.get(r.date) ?? 0) + r.tokens);
    return [...byDate.entries()].sort((a, b) => (a[0] < b[0] ? -1 : 1)).map(([, v]) => v);
  });

  // ── quads ──
  const cachePct = $derived.by(() => {
    if (!overview) return 0;
    const served = overview.cache_read_tokens;
    const fresh = overview.input_tokens;
    return served + fresh > 0 ? (served / (served + fresh)) * 100 : 0;
  });

  // ── stacked daily chart ──
  const dailyDates = $derived([...new Set(daily.map((r) => r.date))].sort());
  const dailySources = $derived(
    SOURCE_ORDER.filter((s) => daily.some((r) => r.source === s)).concat(
      [...new Set(daily.map((r) => r.source))].filter((s) => !SOURCE_ORDER.includes(s)),
    ),
  );

  const dailyOption = $derived.by(() => {
    if (!daily.length) return undefined;
    const map = new Map(daily.map((r) => [`${r.date}|${r.source}`, r.tokens]));
    return {
      backgroundColor: 'transparent',
      ...ANIM,
      animationDelay: (idx: number) => idx * 90,
      tooltip: {
        trigger: 'axis',
        ...TOOLTIP,
        valueFormatter: (v: unknown) => fmtTokens(Number(v)),
      },
      grid: { left: 0, right: 0, top: 8, bottom: 0 },
      xAxis: { type: 'category', data: dailyDates, show: false },
      yAxis: { type: 'value', show: false, min: 0 },
      series: dailySources.map((s, i) =>
        stackedBand(sourceLabel(s), dailyDates.map((d) => map.get(`${d}|${s}`) ?? 0), sourceColor(s), {
          delay: i * 90,
        }),
      ),
    } satisfies EChartsOption;
  });

  // month labels for the custom axis row under the plot
  const monthMarks = $derived.by(() => {
    if (!dailyDates.length) return [] as string[];
    const marks: string[] = [];
    let lastMonth = '';
    for (const d of dailyDates) {
      const m = d.slice(0, 7);
      if (m !== lastMonth) {
        marks.push(new Date(d + 'T00:00:00').toLocaleDateString(undefined, { month: 'short' }).toUpperCase());
        lastMonth = m;
      }
    }
    return marks;
  });

  // second half of the window vs the first half
  const deltaPct = $derived.by(() => {
    const n = dailyTotals.length;
    if (n < 8) return null;
    const mid = Math.floor(n / 2);
    const first = dailyTotals.slice(0, mid).reduce((a, b) => a + b, 0);
    const second = dailyTotals.slice(mid).reduce((a, b) => a + b, 0);
    if (first === 0) return null;
    return ((second - first) / first) * 100;
  });

  // ── lower split ──
  const maxModelTokens = $derived(Math.max(1, ...models.map((m) => m.tokens)));

  const harnessCells = $derived.by(() => {
    const ov = overview;
    if (!ov?.total_tokens) return [];
    return [...ov.by_source]
      .sort((a, b) => b.tokens - a.tokens)
      .map((s) => ({
        label: sourceLabel(s.source),
        pct: (s.tokens / ov.total_tokens) * 100,
      }));
  });

  // ── NOTE 01: burst concentration over the loaded window ──
  const burst = $derived.by(() => {
    const active = dailyTotals.filter((v) => v > 0);
    const total = active.reduce((a, b) => a + b, 0);
    if (active.length < 10 || total === 0) return null;
    const topN = Math.max(1, Math.ceil(active.length * 0.1));
    const topSum = [...active].sort((a, b) => b - a).slice(0, topN).reduce((a, b) => a + b, 0);
    return { share: Math.round((topSum / total) * 100), topN, days: active.length };
  });
</script>

{#if error}
  <p class="error">{error}</p>
{:else if !overview}
  <div class="loading">loading your stats…</div>
{:else}
  <!-- hero band -->
  <section class="band">
    <div class="hero reg up">
      <div class="kick"><span>Total tokens · all time</span></div>
      <div class="n">
        <AnimatedNumber value={overview.total_tokens} format={(n) => fmtTokensSplit(n).value} />
        {#if fmtTokensSplit(overview.total_tokens).unit}
          <u>{fmtTokensSplit(overview.total_tokens).unit}</u>
        {/if}
      </div>
      <div class="sp">
        {#if dailyTotals.length > 1}
          <Spark values={dailyTotals} width={460} height={30} delay={350} />
        {/if}
      </div>
    </div>
    <div class="quad">
      <div class="q org up" style="animation-delay:70ms">
        <div class="k">Est. cost</div>
        <div class="v"><AnimatedNumber value={overview.cost_usd ?? 0} format={fmtCost} /></div>
        <div class="h">api-equivalent estimate</div>
      </div>
      <div class="q up" style="animation-delay:140ms">
        <div class="k">Sessions</div>
        <div class="v"><AnimatedNumber value={overview.sessions} /></div>
        <div class="h">{overview.events.toLocaleString()} model calls</div>
      </div>
      <div class="q acd up" style="animation-delay:210ms">
        <div class="k">Streak</div>
        <div class="v"><AnimatedNumber value={overview.current_streak} format={(n) => `${Math.round(n)}d`} /></div>
        <div class="h">longest {overview.longest_streak}d ever</div>
      </div>
      <div class="q cyn up" style="animation-delay:280ms">
        <div class="k">Cache served</div>
        <div class="v"><AnimatedNumber value={cachePct} format={(n) => `${Math.round(n)}%`} /></div>
        <div class="h">{fmtTokens(overview.cache_read_tokens)} tokens re-served</div>
      </div>
    </div>
  </section>

  <!-- stacked daily chart -->
  <section class="daily">
    <div class="hd">
      <h3>Daily tokens by provider — last 150 days</h3>
      <div class="rt">
        {#if deltaPct !== null}
          <b>{deltaPct >= 0 ? '▲' : '▼'} {Math.abs(deltaPct).toFixed(0)}% VS PREV</b>
        {/if}
      </div>
    </div>
    <div class="legend">
      {#each dailySources as s}
        <span><i style="background:{sourceColor(s)}"></i>{sourceLabel(s)}</span>
      {/each}
    </div>
    <div class="plot">
      {#if dailyOption}
        <Chart option={dailyOption} height="fill" />
      {:else}
        <div class="loading">no usage recorded yet</div>
      {/if}
    </div>
    {#if monthMarks.length}
      <div class="ax">
        {#each monthMarks as m}<span>{m}</span>{/each}
      </div>
    {/if}
  </section>

  <!-- lower split: models + harnesses -->
  <section class="low">
    <div class="bars">
      <h3><span>Top models · 30 days</span></h3>
      {#if models.length}
        {#each models as m, i}
          <div class="rankbar up" style="animation-delay:{180 + i * 60}ms">
            <span class="chip" style="background:{MODEL_PALETTE[i % MODEL_PALETTE.length]}">{i + 1}</span>
            <span class="nm" title={m.model}>{m.model}</span>
            <span class="tr">
              <div
                class="gw"
                style="width:{Math.max(2, Math.round((m.tokens / maxModelTokens) * 100))}%;background:{MODEL_PALETTE[i % MODEL_PALETTE.length]};animation-delay:{240 + i * 60}ms"
              ></div>
            </span>
            <b><AnimatedNumber value={m.tokens} format={fmtTokens} duration={1100} /></b>
          </div>
        {/each}
      {:else}
        <div class="loading">nothing yet</div>
      {/if}
    </div>
    <div class="srcs">
      <h3><span>Harnesses</span></h3>
      <div class="g">
        {#each harnessCells as c, i}
          <div class="up" style="animation-delay:{240 + i * 60}ms">
            <div class="n"><AnimatedNumber value={c.pct} format={(x) => `${Math.round(x)}%`} /></div>
            <div class="l">{c.label}</div>
          </div>
        {/each}
      </div>
    </div>
  </section>

  <!-- note band -->
  {#if burst}
    <div class="noteband up" style="animation-delay:420ms">
      <span class="fg">NOTE</span>
      <p>You work in bursts — <b>{burst.share}%</b> of your tokens land on just <b>{burst.topN} {burst.topN === 1 ? 'day' : 'days'}</b> of the last {burst.days} active.</p>
    </div>
  {/if}
{/if}

<style>
  .band {
    display: grid;
    grid-template-columns: 1.06fr 0.94fr;
    border-bottom: 2px solid var(--ink);
    margin-bottom: 0;
  }

  .hero { padding: clamp(16px, 1.6vh, 26px) clamp(22px, 1.8vw, 40px) clamp(14px, 1.4vh, 24px); border-right: 2px solid var(--ink); position: relative; overflow: hidden; }
  .hero .kick {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    font-family: var(--font-ui);
    font-weight: 600;
    font-size: clamp(10px, 0.75vw, 13px);
    line-height: 1;
    letter-spacing: 1.7px;
    text-transform: uppercase;
  }
  .hero .n {
    font-family: var(--font-disp);
    font-weight: 400;
    font-size: clamp(72px, 6.2vw, 170px);
    line-height: 0.8;
    letter-spacing: -2px;
    margin: clamp(15px, 2vh, 30px) 0 0;
    display: flex;
    align-items: flex-start;
    font-variant-numeric: tabular-nums;
  }
  .hero .n u { text-decoration: none; font-size: clamp(28px, 2.4vw, 64px); color: var(--org); margin-left: clamp(4px, 0.4vw, 10px); margin-top: clamp(6px, 0.6vw, 16px); }
  .hero .sp { margin-top: clamp(12px, 1.4vh, 22px); height: clamp(30px, 2.4vw, 56px); }
  .hero .sp :global(svg) { width: 100%; height: 100%; display: block; }

  .quad { display: grid; grid-template-columns: 1fr 1fr; grid-template-rows: 1fr 1fr; }
  .q { padding: clamp(13px, 1.5vh, 24px) clamp(17px, 1.4vw, 28px); border-right: 2px solid var(--ink); border-bottom: 2px solid var(--ink); position: relative; }
  .q:nth-child(2n) { border-right: none; }
  .q:nth-child(n + 3) { border-bottom: none; }
  .q .k { font: 600 clamp(9px, 0.7vw, 12px)/1 var(--font-ui); letter-spacing: 1.4px; text-transform: uppercase; opacity: 0.6; }
  .q .v { font: 400 clamp(32px, 2.7vw, 58px)/1 var(--font-disp); margin-top: clamp(8px, 1vh, 16px); letter-spacing: -1px; font-variant-numeric: tabular-nums; }
  .q .h { font: 400 clamp(10px, 0.75vw, 13px)/1.3 var(--font-mono); margin-top: clamp(7px, 0.8vh, 13px); opacity: 0.52; }
  .q.org { background: var(--org); color: #fff; }
  .q.org .k, .q.org .h { opacity: 0.82; }
  .q.cyn { background: var(--cyn); }
  .q.acd { background: var(--acd); }

  .daily {
    display: flex;
    flex-direction: column;
    padding: clamp(13px, 1.3vh, 22px) clamp(22px, 1.8vw, 40px) clamp(10px, 1vh, 16px);
    border-bottom: 2px solid var(--ink);
    flex: 1 1 auto;
    min-height: 250px;
  }
  .daily .hd { display: flex; justify-content: space-between; align-items: baseline; flex: none; }
  .daily .hd h3 { font: 600 clamp(11px, 0.85vw, 15px)/1 var(--font-ui); letter-spacing: 1.6px; text-transform: uppercase; margin: 0; }
  .daily .hd .rt { display: flex; gap: 14px; align-items: baseline; }
  .daily .hd .rt b { font: 500 clamp(10px, 0.75vw, 13px)/1 var(--font-mono); letter-spacing: 1px; color: var(--org); }
  .daily .legend { display: flex; gap: 16px; margin-top: 10px; flex: none; flex-wrap: wrap; }
  .daily .legend span {
    display: flex;
    align-items: center;
    gap: 5px;
    font: 500 10px/1 var(--font-mono);
    letter-spacing: 0.8px;
    text-transform: uppercase;
    opacity: 0.7;
  }
  .daily .legend i { width: 10px; height: 10px; display: inline-block; flex: none; }
  .daily .plot {
    flex: 1;
    min-height: 0;
    margin-top: 8px;
    display: flex;
    flex-direction: column;
  }
  .daily .ax {
    display: flex;
    justify-content: space-between;
    flex: none;
    padding-top: 6px;
    border-top: 2px solid var(--ink);
    font: 500 9px/1 var(--font-mono);
    letter-spacing: 1.4px;
    opacity: 0.5;
  }

  .low { display: grid; grid-template-columns: 1.5fr 1fr; }
  .low .bars { padding: clamp(13px, 1.3vh, 22px) clamp(22px, 1.8vw, 40px) clamp(15px, 1.5vh, 24px); border-right: 2px solid var(--ink); display: flex; flex-direction: column; }
  .low h3 {
    font: 600 clamp(11px, 0.85vw, 15px)/1 var(--font-ui);
    letter-spacing: 1.6px;
    text-transform: uppercase;
    margin: 0 0 12px;
    display: flex;
    justify-content: space-between;
  }
  .srcs { padding: clamp(13px, 1.3vh, 22px) clamp(18px, 1.4vw, 30px) clamp(15px, 1.5vh, 24px); display: flex; flex-direction: column; }
  .srcs .g { display: grid; grid-template-columns: 1fr 1fr; grid-auto-rows: 1fr; gap: 5px; flex: 1; }
  .srcs .g > div {
    border: 2px solid var(--ink);
    padding: clamp(8px, 1vh, 16px) clamp(10px, 0.8vw, 16px);
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    gap: 6px;
  }
  .srcs .g .n { font: 400 clamp(20px, 1.7vw, 38px)/1 var(--font-disp); letter-spacing: -0.5px; font-variant-numeric: tabular-nums; }
  .srcs .g .l {
    font: 500 clamp(8px, 0.65vw, 11px)/1.3 var(--font-mono);
    letter-spacing: 0.9px;
    text-transform: uppercase;
    opacity: 0.55;
  }

  .noteband {
    border-left: none;
    border-right: none;
    border-bottom: none;
    border-top: 2px solid var(--ink);
    margin-top: 0;
  }

  @media (max-width: 900px) {
    .band { grid-template-columns: 1fr; }
    .hero { border-right: none; border-bottom: 2px solid var(--ink); }
    .hero .n { font-size: 72px; }
    .low { grid-template-columns: 1fr; }
    .low .bars { border-right: none; border-bottom: 2px solid var(--ink); }
  }
</style>
