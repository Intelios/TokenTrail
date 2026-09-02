<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import type { EChartsOption } from 'echarts';
  import Chart from '$lib/Chart.svelte';
  import AnimatedNumber from '$lib/AnimatedNumber.svelte';
  import Spark from '$lib/Spark.svelte';
  import { api, type DailyModelRow, type FamilyStatsRow } from '$lib/api';
  import {
    fmtCost,
    fmtTokens,
    fmtTokensSplit,
    sourceSwatch,
    sourceLabel,
    familyColor,
    familySwatch,
    familyFlat,
  } from '$lib/format';
  import { TOOLTIP, ANIM, donutSeries } from '$lib/chartTheme';
  import { readPref, writePref } from '$lib/prefs';

  const RANGES: [number, string][] = [
    [7, '7D'],
    [30, '30D'],
    [90, '90D'],
    [3650, 'ALL'],
  ];

  const METRICS: ['tokens' | 'cost' | 'calls', string][] = [
    ['tokens', 'Tokens'],
    ['cost', 'Cost'],
    ['calls', 'Calls'],
  ];

  const RANGE_LABEL: Record<number, string> = {
    7: '7 days',
    30: '30 days',
    90: '90 days',
    3650: 'all time',
  };

  const PREF_DAYS = 'tt.families.days';
  const PREF_METRIC = 'tt.families.metric';

  let days = $state(readPref(PREF_DAYS, 90, (v) => RANGES.some(([d]) => d === v)));
  let metric = $state(
    readPref<'tokens' | 'cost' | 'calls'>(PREF_METRIC, 'tokens', (v) => METRICS.some(([m]) => m === v)),
  );
  let families = $state<FamilyStatsRow[]>([]);
  let trendRows = $state<DailyModelRow[]>([]);
  let expanded = $state<Set<string>>(new Set());
  let error = $state('');

  function metricVal(f: FamilyStatsRow): number {
    if (metric === 'tokens') return f.tokens;
    if (metric === 'cost') return f.cost_usd ?? 0;
    return f.events;
  }

  function metricValModel(m: { tokens: number; cost_usd: number | null; events: number }): number {
    if (metric === 'tokens') return m.tokens;
    if (metric === 'cost') return m.cost_usd ?? 0;
    return m.events;
  }

  function fmtMetric(n: number): string {
    if (metric === 'tokens') return fmtTokens(n);
    if (metric === 'cost') return fmtCost(n);
    return n.toLocaleString();
  }

  async function load() {
    try {
      const [fam, trends] = await Promise.all([api.familyStats(days), api.dailyByModel(90)]);
      families = fam;
      trendRows = trends;
      error = '';
    } catch (e) {
      error = String(e);
    }
  }

  $effect(() => {
    days;
    load();
  });

  // remember the chosen range / metric across visits
  $effect(() => {
    writePref(PREF_DAYS, days);
    writePref(PREF_METRIC, metric);
  });

  onMount(() => {
    load();
    const h = () => load();
    window.addEventListener('tt-sync', h);
    return () => window.removeEventListener('tt-sync', h);
  });

  const sorted = $derived(
    metric === 'tokens'
      ? [...families].sort((a, b) => b.tokens - a.tokens)
      : metric === 'cost'
        ? [...families].sort((a, b) => (b.cost_usd ?? 0) - (a.cost_usd ?? 0))
        : [...families].sort((a, b) => b.events - a.events),
  );

  const totalTokens = $derived(families.reduce((s, f) => s + f.tokens, 0));
  const totalCost = $derived(families.reduce((s, f) => s + (f.cost_usd ?? 0), 0));
  const totalEvents = $derived(families.reduce((s, f) => s + f.events, 0));
  const totalModels = $derived(families.reduce((s, f) => s + f.models.length, 0));
  const totalMetric = $derived(sorted.reduce((s, f) => s + metricVal(f), 0));
  const costliest = $derived(
    [...families].sort((a, b) => (b.cost_usd ?? 0) - (a.cost_usd ?? 0))[0],
  );
  const metricLabel = $derived(
    metric === 'tokens' ? 'tokens' : metric === 'cost' ? 'cost' : 'calls',
  );

  const top6 = $derived(sorted.slice(0, 6));
  const maxTop6 = $derived(Math.max(1, ...top6.map(metricVal)));

  // per-model 90-day series for sparklines
  const trendByModel = $derived.by(() => {
    const map = new Map<string, Map<string, number>>();
    for (const r of trendRows) {
      let m = map.get(r.model);
      if (!m) {
        m = new Map();
        map.set(r.model, m);
      }
      m.set(r.date, r.tokens);
    }
    const out = new Map<string, number[]>();
    for (const [model, m] of map) {
      out.set(
        model,
        [...m.entries()].sort((a, b) => (a[0] < b[0] ? -1 : 1)).map(([, v]) => v),
      );
    }
    return out;
  });

  const donutOption = $derived.by(() => {
    if (!sorted.length) return undefined;
    const top8 = sorted.slice(0, 8);
    const otherVal = sorted.slice(8).reduce((s, f) => s + metricVal(f), 0);
    const data = top8.map((f, i) => ({
      name: f.family,
      value: metricVal(f),
      itemStyle: { color: familyColor(f.family, i) },
    }));
    if (otherVal > 0)
      data.push({ name: 'Other', value: otherVal, itemStyle: { color: '#8a8578' } });
    return {
      backgroundColor: 'transparent',
      ...ANIM,
      tooltip: {
        ...TOOLTIP,
        formatter: (p: any) =>
          `${p.name}<br/>${fmtMetric(Number(p.value ?? 0))} (${p.percent}%)`,
      },
      series: [donutSeries(data)],
    } satisfies EChartsOption;
  });

  const topFamilyShare = $derived.by(() => {
    if (!sorted.length || !totalMetric) return null;
    const top = sorted[0];
    const val = metricVal(top);
    if (!val) return null;
    return {
      family: top.family,
      pct: Math.round((val / totalMetric) * 100),
      label: metric === 'tokens' ? 'tokens' : metric === 'cost' ? 'estimated spend' : 'model calls',
    };
  });

  function toggle(family: string) {
    const next = new Set(expanded);
    if (next.has(family)) next.delete(family);
    else next.add(family);
    expanded = next;
  }

  function modelUrl(name: string): string {
    return '/models/' + encodeURIComponent(name);
  }
</script>

<div class="fframe">
  <!-- header -->
  <div class="thd">
    <div class="up">
      <h1>Families</h1>
      <div class="sub">
        {families.length} families · {totalModels} models · {totalEvents.toLocaleString()} calls · {RANGE_LABEL[days]}
      </div>
    </div>
    <div class="pillsrow up" style="animation-delay:80ms">
      <div class="pills">
        {#each RANGES as [d, label]}
          <button class="pill" class:on={days === d} onclick={() => (days = d)}>{label}</button>
        {/each}
      </div>
      <div class="pills">
        {#each METRICS as [m, label]}
          <button class="pill" class:on={metric === m} onclick={() => (metric = m)}>{label}</button>
        {/each}
      </div>
    </div>
  </div>

  {#if error}
    <div class="errpad"><p class="error">{error}</p></div>
  {:else if !families.length}
    <div class="loading">no models recorded yet</div>
  {:else}
    <!-- stat cards -->
    <div class="mcards">
      <div class="mc up">
        <div class="k">Families used</div>
        <div class="v"><AnimatedNumber value={families.length} /></div>
        <div class="h">{totalModels} models total</div>
      </div>
      <div class="mc hl up" style="animation-delay:60ms">
        <div class="k">Top family</div>
        <div class="v name ell" title={sorted[0]?.family}>{sorted[0]?.family ?? '—'}</div>
        <div class="h">
          {totalMetric ? ((metricVal(sorted[0]) / totalMetric) * 100).toFixed(1) : 0}% of {metricLabel}
        </div>
      </div>
      <div class="mc up" style="animation-delay:120ms">
        <div class="k">Costliest</div>
        <div class="v name ell" title={costliest?.family}>{costliest?.family ?? '—'}</div>
        <div class="h">{fmtCost(costliest?.cost_usd ?? null)} est.</div>
      </div>
      <div class="mc up" style="animation-delay:180ms">
        <div class="k">Est. cost</div>
        <div class="v"><AnimatedNumber value={totalCost} format={fmtCost} /></div>
        <div class="h">{fmtTokens(totalTokens)} tokens</div>
      </div>
      <div class="mc up" style="animation-delay:240ms">
        <div class="k">Avg tok / call</div>
        <div class="v">
          <AnimatedNumber
            value={totalEvents ? totalTokens / totalEvents : 0}
            format={(n) => fmtTokensSplit(n).value}
          />
          {#if totalEvents}
            <span class="unit">{fmtTokensSplit(totalTokens / totalEvents).unit || ' '}</span>
          {/if}
        </div>
        <div class="h">{totalEvents.toLocaleString()} calls total</div>
      </div>
    </div>

    <!-- bars + donut -->
    <div class="mcharts">
      <div class="mbars">
        <h3>
          <span>
            {metricLabel === 'calls'
              ? 'Call share'
              : metricLabel === 'cost'
                ? 'Cost share'
                : 'Token share'} — top {Math.min(6, sorted.length)}
          </span>
        </h3>
        {#each top6 as f, i}
          {@const color = familySwatch(f.family, i)}
          <div class="rankbar up" style="animation-delay:{i * 50}ms">
            <span class="chip" style="background:{color}">{i + 1}</span>
            <span class="nm" title={f.family}>{f.family}</span>
            <span class="tr">
              <div
                class="gw"
                style="width:{Math.max(2, Math.round((metricVal(f) / maxTop6) * 100))}%;background:{color};animation-delay:{100 + i * 50}ms"
              ></div>
            </span>
            <b><AnimatedNumber value={metricVal(f)} format={fmtMetric} duration={1100} /></b>
            <span class="pct">{totalMetric ? ((metricVal(f) / totalMetric) * 100).toFixed(0) : 0}%</span>
          </div>
        {/each}
      </div>
      <div class="donut">
        <h3><span>Share of {metricLabel}</span></h3>
        {#if donutOption}
          <Chart option={donutOption} height={170} />
        {/if}
      </div>
    </div>

    <!-- family sections -->
    <div class="fw">
      {#each sorted as f, fi}
        {@const color = familySwatch(f.family, fi)}
        {@const flat = familyFlat(f.family, fi)}
        {@const isOpen = expanded.has(f.family)}
        <div class="fam" class:open={isOpen}>
          <!-- family header -->
          <button class="fhead up" style="animation-delay:{fi * 45}ms" onclick={() => toggle(f.family)}>
            <span class="chip" style="background:{color}">{fi + 1}</span>
            <span class="fnm">{f.family}</span>
            <span class="fcnt">{f.models.length} model{f.models.length !== 1 ? 's' : ''}</span>
            <span class="ftags">
              {#each f.sources as s}
                <span class="stag"><i style="background:{sourceSwatch(s)}"></i>{sourceLabel(s)}</span>
              {/each}
            </span>
            <span class="fstat">{fmtTokens(f.tokens)}</span>
            <span class="fstat">{fmtCost(f.cost_usd)}</span>
            <span class="fstat">{f.events.toLocaleString()} calls</span>
            <span class="fshare">
              <span class="shbar">
                <div
                  class="gw"
                  style="width:{totalMetric ? Math.max(2, (metricVal(f) / totalMetric) * 100) : 0}%;background:{color};animation-delay:{fi * 45}ms"
                ></div>
              </span>
              <span class="pct">{totalMetric ? ((metricVal(f) / totalMetric) * 100).toFixed(0) : 0}%</span>
            </span>
            <span class="chev" class:rot={isOpen}>▼</span>
          </button>

          <!-- member models -->
          {#if isOpen}
            <div class="fbody">
              <table>
                <thead>
                  <tr>
                    <th>#</th>
                    <th>Model</th>
                    <th>Sources</th>
                    <th class="num">Calls</th>
                    <th class="num">Tokens</th>
                    <th class="num">In / Out</th>
                    <th class="num">Avg/call</th>
                    <th>90d trend</th>
                    <th class="num">Est. cost</th>
                    <th class="num">Share</th>
                  </tr>
                </thead>
                <tbody>
                  {#each f.models as m, mi}
                    {@const trend = trendByModel.get(m.model)}
                    {@const mTotal = metricValModel(m)}
                    {@const mMax = f.models.reduce((s, x) => s + metricValModel(x), 0)}
                    <tr
                      class="up"
                      style="animation-delay:{mi * 40}ms;cursor:pointer"
                      onclick={() => goto(modelUrl(m.model))}
                    >
                      <td class="rk">
                        <span class="chip" style="background:{color};opacity:0.65">{mi + 1}</span>
                      </td>
                      <td>
                        <a class="modellink" href={modelUrl(m.model)} onclick={(e) => e.stopPropagation()}>
                          {m.model}
                        </a>
                      </td>
                      <td class="srccount">{m.sources.length} src</td>
                      <td class="num">{m.events.toLocaleString()}</td>
                      <td class="num">
                        {fmtTokens(m.tokens)}
                        <div class="breakdown">{m.sessions.toLocaleString()} sessions</div>
                      </td>
                      <td class="num inout">
                        {fmtTokens(m.input_tokens)} in<br />{fmtTokens(m.output_tokens)} out
                      </td>
                      <td class="num">{m.events ? fmtTokens(m.tokens / m.events) : '—'}</td>
                      <td>
                        {#if trend && trend.length > 1}
                          <Spark values={trend} width={94} height={20} color={flat} delay={200 + mi * 40} />
                        {:else}
                          <span class="dim">—</span>
                        {/if}
                      </td>
                      <td class="num">{fmtCost(m.cost_usd)}</td>
                      <td class="num">
                        <span class="shbar">
                          <div
                            class="gw"
                            style="width:{mTotal && mMax ? Math.max(2, (mTotal / mMax) * 100) : 0}%;background:{color};animation-delay:{mi * 40}ms"
                          ></div>
                        </span>
                      </td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            </div>
          {/if}
        </div>
      {/each}
    </div>

    <!-- note band -->
    {#if topFamilyShare !== null}
      <div class="noteband up" style="animation-delay:400ms">
        <span class="fg">NOTE</span>
        <p>
          <b>{topFamilyShare.family}</b> accounts for <b>{topFamilyShare.pct}%</b>
          of your {topFamilyShare.label} in this period. Costs are API-equivalent estimates
          from bundled list prices, not actual subscription spend.
        </p>
      </div>
    {/if}
  {/if}
</div>

<style>
  .fframe {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .thd {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 16px;
    padding: 15px clamp(22px, 1.8vw, 40px) 14px;
    border-bottom: 2px solid var(--ink);
    flex-wrap: wrap;
  }
  .thd .sub {
    font: 400 11px/1 var(--font-mono);
    opacity: 0.55;
    margin-top: 6px;
    letter-spacing: 0.6px;
  }
  .pillsrow {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
  }

  .errpad {
    padding: 20px 22px;
  }

  .mcards {
    display: grid;
    grid-template-columns: repeat(5, 1fr);
    border-bottom: 2px solid var(--ink);
  }
  .mc {
    padding: clamp(12px, 1.2vh, 22px) clamp(16px, 1.2vw, 26px);
    border-right: 2px solid var(--ink);
    min-width: 0;
  }
  .mc:last-child {
    border-right: none;
  }
  .mc .k {
    font: 600 clamp(9px, 0.7vw, 12px)/1 var(--font-ui);
    letter-spacing: 1.3px;
    text-transform: uppercase;
    opacity: 0.55;
  }
  .mc .v {
    font: 400 clamp(26px, 2.1vw, 44px)/1 var(--font-disp);
    margin-top: 8px;
    letter-spacing: -1px;
    font-variant-numeric: tabular-nums;
  }
  .mc .v.name {
    font-size: clamp(16px, 1.3vw, 27px);
    line-height: 1.25;
    margin-top: 10px;
    letter-spacing: 0.2px;
    padding-bottom: 3px;
  }
  .mc .v .unit {
    color: var(--org);
    font-size: 0.55em;
  }
  .mc .h {
    font: 400 clamp(10px, 0.75vw, 12px)/1.3 var(--font-mono);
    margin-top: 7px;
    opacity: 0.48;
  }
  .mc.hl {
    background: var(--org);
    color: #fff;
  }
  .mc.hl .k,
  .mc.hl .h {
    opacity: 0.82;
  }

  .mcharts {
    display: grid;
    grid-template-columns: 1.3fr 0.7fr;
    border-bottom: 2px solid var(--ink);
  }
  .mbars {
    padding: 14px clamp(22px, 1.8vw, 40px) 15px;
    border-right: 2px solid var(--ink);
  }
  .mbars h3,
  .donut h3 {
    font: 600 11px/1 var(--font-ui);
    letter-spacing: 1.6px;
    text-transform: uppercase;
    margin: 0 0 11px;
    display: flex;
    justify-content: space-between;
  }
  .donut {
    padding: 14px 18px 10px;
    display: flex;
    flex-direction: column;
  }
  .donut h3 {
    align-self: stretch;
  }

  /* --- family sections --- */
  .fw {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }

  .fam {
    border-bottom: 2px solid var(--ink);
  }
  .fam:last-child {
    border-bottom: none;
  }

  .fhead {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 12px clamp(22px, 1.8vw, 40px);
    background: none;
    border: none;
    border-radius: 0;
    cursor: pointer;
    font: inherit;
    color: inherit;
    text-align: left;
    text-transform: none;
    letter-spacing: normal;
  }
  .fhead:hover {
    background: rgba(13, 13, 11, 0.04);
  }

  .fnm {
    font: 400 clamp(20px, 1.6vw, 32px)/1 var(--font-disp);
    text-transform: uppercase;
    letter-spacing: -0.5px;
    flex: none;
  }

  .fcnt {
    font: 400 10.5px var(--font-mono);
    opacity: 0.5;
    flex: none;
  }

  .ftags {
    display: flex;
    gap: 5px;
    flex-wrap: wrap;
    flex: 1;
    min-width: 0;
  }

  .stag {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font: 600 9px/1 var(--font-mono);
    letter-spacing: 0.6px;
    text-transform: uppercase;
    padding: 3px 6px;
    border: 1px solid var(--hair);
    background: rgba(13, 13, 11, 0.05);
    color: var(--ink);
  }
  .stag i {
    width: 6px;
    height: 6px;
    display: inline-block;
    flex: none;
  }

  .fstat {
    font: 500 11.5px var(--font-mono);
    white-space: nowrap;
    flex: none;
    opacity: 0.7;
  }

  .fshare {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: none;
  }

  .chev {
    font: 400 10px var(--font-mono);
    opacity: 0.4;
    transition: transform 0.2s ease;
    flex: none;
  }
  .chev.rot {
    transform: rotate(180deg);
  }

  /* member model table */
  .fbody {
    border-top: 2px solid var(--hair);
    background: rgba(13, 13, 11, 0.018);
  }
  .fbody table {
    width: 100%;
  }
  .fbody td {
    padding: clamp(7px, 0.9vh, 14px) 12px;
  }

  .rk {
    width: 34px;
    text-align: center;
    padding-left: 6px !important;
    padding-right: 6px !important;
  }
  .srccount {
    font: 500 11px var(--font-mono);
    opacity: 0.6;
  }
  .inout {
    font-size: 10.5px;
    line-height: 1.5;
  }
  .breakdown {
    font: 400 9px/1 var(--font-mono);
    opacity: 0.45;
    margin-top: 3px;
    letter-spacing: 0.3px;
  }
  .dim {
    opacity: 0.35;
  }

  .shbar {
    height: 9px;
    background: var(--hair);
    width: 64px;
    display: inline-block;
    vertical-align: middle;
  }
  .shbar > div {
    height: 100%;
  }

  .modellink {
    color: var(--ink);
    font-weight: 600;
    font-size: 12.5px;
    text-decoration: none;
  }
  .modellink:hover {
    color: var(--org);
    text-decoration: underline;
  }

  .noteband {
    border-left: none;
    border-right: none;
    border-bottom: none;
    border-top: 2px solid var(--ink);
  }

  @media (max-width: 900px) {
    .mcards {
      grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
    }
    .mc {
      border-bottom: 2px solid var(--ink);
    }
    .mcharts {
      grid-template-columns: 1fr;
    }
    .mbars {
      border-right: none;
      border-bottom: 2px solid var(--ink);
    }
    .ftags {
      display: none;
    }
    .fstat:last-of-type {
      display: none;
    }
  }
</style>
