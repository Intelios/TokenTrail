<script lang="ts">
  import { onMount } from 'svelte';
  import type { EChartsOption } from 'echarts';
  import Chart from '$lib/Chart.svelte';
  import { api, type DailyRow, type ModelRow, type Overview } from '$lib/api';
  import { fmtCost, fmtDate, fmtTokens, sourceColor, sourceLabel, MODEL_PALETTE } from '$lib/format';

  let overview = $state<Overview | null>(null);
  let daily = $state<DailyRow[]>([]);
  let models = $state<ModelRow[]>([]);
  let error = $state('');

  async function load() {
    try {
      const [o, d, m] = await Promise.all([api.overview(), api.daily(90), api.byModel(30)]);
      overview = o;
      daily = d;
      models = m.slice(0, 6);
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

  const dailyOption = $derived.by(() => {
    if (!daily.length) return undefined;
    const dates = [...new Set(daily.map((r) => r.date))].sort();
    const sources = [...new Set(daily.map((r) => r.source))];
    const map = new Map(daily.map((r) => [`${r.date}|${r.source}`, r.tokens]));
    return {
      backgroundColor: 'transparent',
      tooltip: {
        trigger: 'axis',
        backgroundColor: '#131828',
        borderColor: '#232b41',
        textStyle: { color: '#e2e8f0' },
        valueFormatter: (v: unknown) => fmtTokens(Number(v)),
      },
      legend: { textStyle: { color: '#8b95ab' }, top: 0, icon: 'roundRect' },
      grid: { left: 8, right: 12, top: 32, bottom: 0, containLabel: true },
      xAxis: {
        type: 'category',
        data: dates,
        axisLine: { lineStyle: { color: '#232b41' } },
        axisLabel: { color: '#8b95ab' },
      },
      yAxis: {
        type: 'value',
        axisLabel: { color: '#8b95ab', formatter: (v: number) => fmtTokens(v) },
        splitLine: { lineStyle: { color: 'rgba(35,43,65,0.5)' } },
      },
      series: sources.map((s) => ({
        name: sourceLabel(s),
        type: 'line',
        stack: 'total',
        areaStyle: { opacity: 0.35 },
        smooth: true,
        symbol: 'none',
        lineStyle: { width: 1.5 },
        itemStyle: { color: sourceColor(s) },
        emphasis: { focus: 'series' },
        data: dates.map((d) => map.get(`${d}|${s}`) ?? 0),
      })),
    } satisfies EChartsOption;
  });

  const donutOption = $derived.by(() => {
    if (!overview?.by_source.length) return undefined;
    return {
      backgroundColor: 'transparent',
      tooltip: {
        backgroundColor: '#131828',
        borderColor: '#232b41',
        textStyle: { color: '#e2e8f0' },
        valueFormatter: (v: unknown) => fmtTokens(Number(v)),
      },
      series: [
        {
          type: 'pie',
          radius: ['58%', '82%'],
          label: { show: false },
          itemStyle: { borderColor: '#131828', borderWidth: 2 },
          data: overview.by_source.map((s) => ({
            name: sourceLabel(s.source),
            value: s.tokens,
            itemStyle: { color: sourceColor(s.source) },
          })),
        },
      ],
    } satisfies EChartsOption;
  });

  const maxModelTokens = $derived(Math.max(1, ...models.map((m) => m.tokens)));
</script>

<h1>Overview</h1>
<p class="sub">All harnesses, all time — from your own TokenTrail database</p>

{#if error}
  <p class="error">{error}</p>
{:else if !overview}
  <div class="loading">loading your stats…</div>
{:else}
  <div class="cards">
    <div class="card">
      <div class="label">Total tokens</div>
      <div class="value">{fmtTokens(overview.total_tokens)}</div>
      <div class="hint">input + output + cache</div>
    </div>
    <div class="card">
      <div class="label">Output tokens</div>
      <div class="value">{fmtTokens(overview.output_tokens)}</div>
      <div class="hint">{fmtTokens(overview.input_tokens)} input</div>
    </div>
    <div class="card">
      <div class="label">Est. cost</div>
      <div class="value">{fmtCost(overview.cost_usd)}</div>
      <div class="hint">API-equivalent estimate</div>
    </div>
    <div class="card">
      <div class="label">Sessions</div>
      <div class="value">{overview.sessions.toLocaleString()}</div>
      <div class="hint">{overview.events.toLocaleString()} model calls</div>
    </div>
    <div class="card">
      <div class="label">Current streak</div>
      <div class="value">{overview.current_streak}d</div>
      <div class="hint">longest {overview.longest_streak}d</div>
    </div>
    <div class="card">
      <div class="label">Active days</div>
      <div class="value">{overview.active_days}</div>
      <div class="hint">since {fmtDate(overview.first_ts)}</div>
    </div>
  </div>

  <div class="panel">
    <h2>Daily tokens — last 90 days</h2>
    {#if dailyOption}
      <Chart option={dailyOption} height={300} />
    {:else}
      <div class="loading">no usage recorded yet</div>
    {/if}
  </div>

  <div class="grid2">
    <div class="panel">
      <h2>By source — lifetime</h2>
      {#if donutOption}
        <Chart option={donutOption} height={230} />
      {/if}
    </div>
    <div class="panel">
      <h2>Top models — last 30 days</h2>
      {#if models.length}
        <div class="legendlist">
          {#each models as m, i}
            <div class="item">
              <span class="swatch" style="background:{MODEL_PALETTE[i % MODEL_PALETTE.length]}"></span>
              <span class="name">
                {m.model}
                <span class="bar"><div style="width:{Math.round((m.tokens / maxModelTokens) * 100)}%"></div></span>
              </span>
              <span class="val">{fmtTokens(m.tokens)}</span>
            </div>
          {/each}
        </div>
      {:else}
        <div class="loading">nothing yet</div>
      {/if}
      <a href="/models" class="morelink">All models →</a>
    </div>
  </div>
{/if}
