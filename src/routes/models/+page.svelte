<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import type { EChartsOption } from 'echarts';
  import Chart from '$lib/Chart.svelte';
  import AnimatedNumber from '$lib/AnimatedNumber.svelte';
  import Spark from '$lib/Spark.svelte';
  import { api, type DailyModelRow, type ModelStatsRow } from '$lib/api';
  import {
    fmtCost,
    fmtTokens,
    fmtTokensSplit,
    sourceColor,
    sourceLabel,
    MODEL_PALETTE,
  } from '$lib/format';
  import { TOOLTIP, ANIM, donutSeries } from '$lib/chartTheme';

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

  const RANGE_LABEL: Record<number, string> = { 7: '7 days', 30: '30 days', 90: '90 days', 3650: 'all time' };

  let days = $state(90);
  let metric = $state<'tokens' | 'cost' | 'calls'>('tokens');
  let rows = $state<ModelStatsRow[]>([]);
  let trendRows = $state<DailyModelRow[]>([]);
  let error = $state('');

  function metricVal(r: ModelStatsRow): number {
    if (metric === 'tokens') return r.tokens;
    if (metric === 'cost') return r.cost_usd ?? 0;
    return r.events;
  }

  function fmtMetric(n: number): string {
    if (metric === 'tokens') return fmtTokens(n);
    if (metric === 'cost') return fmtCost(n);
    return n.toLocaleString();
  }

  async function load() {
    try {
      const [rs, trends] = await Promise.all([api.modelStats(days), api.dailyByModel(90)]);
      rows = rs;
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

  onMount(() => {
    load();
    const h = () => load();
    window.addEventListener('tt-sync', h);
    return () => window.removeEventListener('tt-sync', h);
  });

  const sorted = $derived(
    metric === 'tokens'
      ? [...rows].sort((a, b) => b.tokens - a.tokens)
      : metric === 'cost'
        ? [...rows].sort((a, b) => (b.cost_usd ?? 0) - (a.cost_usd ?? 0))
        : [...rows].sort((a, b) => b.events - a.events),
  );

  const totalMetric = $derived(sorted.reduce((s, r) => s + metricVal(r), 0));
  const totalTokens = $derived(rows.reduce((s, r) => s + r.tokens, 0));
  const totalCost = $derived(rows.reduce((s, r) => s + (r.cost_usd ?? 0), 0));
  const totalEvents = $derived(rows.reduce((s, r) => s + r.events, 0));
  const totalOutput = $derived(rows.reduce((s, r) => s + r.output_tokens, 0));
  const costliest = $derived([...rows].sort((a, b) => (b.cost_usd ?? 0) - (a.cost_usd ?? 0))[0]);
  const metricLabel = $derived(metric === 'tokens' ? 'tokens' : metric === 'cost' ? 'cost' : 'calls');

  // per-model 90-day series for the trend sparkline column
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
      out.set(model, [...m.entries()].sort((a, b) => (a[0] < b[0] ? -1 : 1)).map(([, v]) => v));
    }
    return out;
  });

  const top6 = $derived(sorted.slice(0, 6));
  const maxTop6 = $derived(Math.max(1, ...top6.map(metricVal)));

  const donutOption = $derived.by(() => {
    if (!sorted.length) return undefined;
    const top8 = sorted.slice(0, 8);
    const otherVal = sorted.slice(8).reduce((s, r) => s + metricVal(r), 0);
    const data = top8.map((r, i) => ({
      name: r.model,
      value: metricVal(r),
      itemStyle: { color: MODEL_PALETTE[i % MODEL_PALETTE.length] },
    }));
    if (otherVal > 0) data.push({ name: 'Other', value: otherVal, itemStyle: { color: '#8a8578' } });
    return {
      backgroundColor: 'transparent',
      ...ANIM,
      tooltip: {
        ...TOOLTIP,
        formatter: (p: any) => `${p.name}<br/>${fmtMetric(Number(p.value ?? 0))} (${p.percent}%)`,
      },
      series: [donutSeries(data)],
    } satisfies EChartsOption;
  });

  const topCostShare = $derived.by(() => {
    if (!costliest || !(costliest.cost_usd ?? 0) || !totalCost) return null;
    return ((costliest.cost_usd ?? 0) / totalCost) * 100;
  });

  function modelUrl(name: string): string {
    return '/models/' + encodeURIComponent(name);
  }
</script>

<div class="mframe">
  <!-- header -->
  <div class="thd">
    <div class="up">
      <h1>Models</h1>
      <div class="sub">{rows.length} models · {totalEvents.toLocaleString()} calls · {RANGE_LABEL[days]}</div>
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
  {:else if !rows.length}
    <div class="loading">no models recorded yet</div>
  {:else}
    <!-- stat cards -->
    <div class="mcards">
      <div class="mc up">
        <div class="k">Models used</div>
        <div class="v"><AnimatedNumber value={rows.length} /></div>
        <div class="h">{totalEvents.toLocaleString()} calls total</div>
      </div>
      <div class="mc hl up" style="animation-delay:60ms">
        <div class="k">Top model</div>
        <div class="v name ell" title={sorted[0]?.model}>{sorted[0]?.model ?? '—'}</div>
        <div class="h">{totalMetric ? ((metricVal(sorted[0]) / totalMetric) * 100).toFixed(1) : 0}% of {metricLabel}</div>
      </div>
      <div class="mc up" style="animation-delay:120ms">
        <div class="k">Costliest</div>
        <div class="v name ell" title={costliest?.model}>{costliest?.model ?? '—'}</div>
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
          <AnimatedNumber value={totalEvents ? totalTokens / totalEvents : 0} format={(n) => fmtTokensSplit(n).value} />
          {#if totalEvents}
            <span class="unit">{fmtTokensSplit(totalTokens / totalEvents).unit || ' '}</span>
          {/if}
        </div>
        <div class="h">{totalTokens ? ((totalOutput / totalTokens) * 100).toFixed(0) : 0}% output</div>
      </div>
    </div>

    <!-- bars + donut -->
    <div class="mcharts">
      <div class="mbars">
        <h3><span>{metricLabel === 'calls' ? 'Call share' : metricLabel === 'cost' ? 'Cost share' : 'Token share'} — top 6</span></h3>
        {#each top6 as r, i}
          <div class="rankbar up" style="animation-delay:{i * 50}ms">
            <span class="chip" style="background:{MODEL_PALETTE[i % MODEL_PALETTE.length]}">{i + 1}</span>
            <span class="nm" title={r.model}>{r.model}</span>
            <span class="tr">
              <div
                class="gw"
                style="width:{Math.max(2, Math.round((metricVal(r) / maxTop6) * 100))}%;background:{MODEL_PALETTE[i % MODEL_PALETTE.length]};animation-delay:{100 + i * 50}ms"
              ></div>
            </span>
            <b><AnimatedNumber value={metricVal(r)} format={fmtMetric} duration={1100} /></b>
            <span class="pct">{totalMetric ? ((metricVal(r) / totalMetric) * 100).toFixed(0) : 0}%</span>
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

    <!-- dense table -->
    <div class="tw">
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
          {#each sorted as r, i}
            {@const trend = trendByModel.get(r.model)}
            {@const color = MODEL_PALETTE[i % MODEL_PALETTE.length]}
            <tr class="up" style="animation-delay:{i * 45}ms; cursor:pointer" onclick={() => goto(modelUrl(r.model))}>
              <td class="rk"><span class="chip" style="background:{color}">{i + 1}</span></td>
              <td>
                <a class="modellink" href={modelUrl(r.model)} onclick={(e) => e.stopPropagation()}>{r.model}</a>
                {#if r.sources.length}
                  <div class="tagrow">
                    {#each r.sources as s}
                      <span class="stag" style="border-color:{sourceColor(s)};color:{sourceColor(s)}">{sourceLabel(s)}</span>
                    {/each}
                  </div>
                {/if}
              </td>
              <td class="srccount">{r.sources.length} src</td>
              <td class="num">{r.events.toLocaleString()}</td>
              <td class="num">
                {fmtTokens(r.tokens)}
                <div class="breakdown">{r.sessions.toLocaleString()} sessions</div>
              </td>
              <td class="num inout">
                {fmtTokens(r.input_tokens)} in<br />{fmtTokens(r.output_tokens)} out
              </td>
              <td class="num">{r.events ? fmtTokens(r.tokens / r.events) : '—'}</td>
              <td>
                {#if trend && trend.length > 1}
                  <Spark values={trend} width={94} height={20} color={color} delay={200 + i * 45} />
                {:else}
                  <span class="dim">—</span>
                {/if}
              </td>
              <td class="num">{fmtCost(r.cost_usd)}</td>
              <td class="num">
                <span class="shbar">
                  <div class="gw" style="width:{totalMetric ? Math.max(2, (metricVal(r) / totalMetric) * 100) : 0}%;background:{color};animation-delay:{i * 45}ms"></div>
                </span>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>

    <!-- note band -->
    {#if topCostShare !== null}
      <div class="noteband up" style="animation-delay:400ms">
        <span class="fg">NOTE 02</span>
        <p>One model is <b>{topCostShare.toFixed(0)}%</b> of your estimated spend. Merging aliases in Settings keeps this list honest.</p>
      </div>
    {/if}
  {/if}
</div>

<style>
  .mframe {
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
  .thd .sub { font: 400 11px/1 var(--font-mono); opacity: 0.55; margin-top: 6px; letter-spacing: 0.6px; }
  .pillsrow { display: flex; gap: 12px; flex-wrap: wrap; }

  .errpad { padding: 20px 22px; }

  .mcards { display: grid; grid-template-columns: repeat(5, 1fr); border-bottom: 2px solid var(--ink); }
  .mc { padding: clamp(12px, 1.2vh, 22px) clamp(16px, 1.2vw, 26px); border-right: 2px solid var(--ink); min-width: 0; }
  .mc:last-child { border-right: none; }
  .mc .k { font: 600 clamp(9px, 0.7vw, 12px)/1 var(--font-ui); letter-spacing: 1.3px; text-transform: uppercase; opacity: 0.55; }
  .mc .v { font: 400 clamp(26px, 2.1vw, 44px)/1 var(--font-disp); margin-top: 8px; letter-spacing: -1px; font-variant-numeric: tabular-nums; }
  .mc .v.name { font-size: clamp(16px, 1.3vw, 27px); margin-top: 12px; letter-spacing: 0.2px; }
  .mc .v .unit { color: var(--org); font-size: 0.55em; }
  .mc .h { font: 400 clamp(10px, 0.75vw, 12px)/1.3 var(--font-mono); margin-top: 7px; opacity: 0.48; }
  .mc.hl { background: var(--org); color: #fff; }
  .mc.hl .k, .mc.hl .h { opacity: 0.82; }

  .mcharts { display: grid; grid-template-columns: 1.3fr 0.7fr; border-bottom: 2px solid var(--ink); }
  .mbars { padding: 14px clamp(22px, 1.8vw, 40px) 15px; border-right: 2px solid var(--ink); }
  .mbars h3, .donut h3 {
    font: 600 11px/1 var(--font-ui);
    letter-spacing: 1.6px;
    text-transform: uppercase;
    margin: 0 0 11px;
    display: flex;
    justify-content: space-between;
  }
  .donut { padding: 14px 18px 10px; display: flex; flex-direction: column; }
  .donut h3 { align-self: stretch; }

  .tw { flex: 1; min-height: 0; overflow: auto; }
  .tw td { padding: clamp(8px, 1vh, 17px) 12px; }

  .rk { width: 34px; text-align: center; padding-left: 6px; padding-right: 6px; }
  .srccount { font: 500 11px var(--font-mono); opacity: 0.6; }
  .inout { font-size: 10.5px; line-height: 1.5; }
  .breakdown { font: 400 9px/1 var(--font-mono); opacity: 0.45; margin-top: 3px; letter-spacing: 0.3px; }
  .dim { opacity: 0.35; }

  .stag {
    font: 500 8px/1 var(--font-mono);
    letter-spacing: 0.6px;
    text-transform: uppercase;
    padding: 2px 5px;
    border: 1.5px solid;
    opacity: 0.75;
  }

  .shbar { height: 9px; background: var(--hair); width: 64px; display: inline-block; vertical-align: middle; }

  .modellink { color: var(--ink); font-weight: 600; font-size: 12.5px; text-decoration: none; }
  .modellink:hover { color: var(--org); text-decoration: underline; }

  .noteband {
    border-left: none;
    border-right: none;
    border-bottom: none;
    border-top: 2px solid var(--ink);
  }

  @media (max-width: 900px) {
    .mcards { grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); }
    .mc { border-bottom: 2px solid var(--ink); }
    .mcharts { grid-template-columns: 1fr; }
    .mbars { border-right: none; border-bottom: 2px solid var(--ink); }
  }
</style>
