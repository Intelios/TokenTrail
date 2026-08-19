<script lang="ts">
  import { onMount } from 'svelte';
  import type { EChartsOption } from 'echarts';
  import Chart from '$lib/Chart.svelte';
  import { api, type HeatmapCell, type HourRow, type Overview } from '$lib/api';
  import { fmtTokens } from '$lib/format';

  let heatmap = $state<HeatmapCell[]>([]);
  let hourly = $state<HourRow[]>([]);
  let overview = $state<Overview | null>(null);
  let error = $state('');

  async function load() {
    try {
      const [h, o, ov] = await Promise.all([api.heatmap(365), api.hourly(), api.overview()]);
      heatmap = h;
      hourly = o;
      overview = ov;
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

  const heatOption = $derived.by(() => {
    if (!heatmap.length) return undefined;
    const end = new Date();
    const start = new Date(end.getTime() - 364 * 86400000);
    const iso = (d: Date) => d.toISOString().slice(0, 10);
    const max = Math.max(...heatmap.map((c) => c.tokens), 1);
    return {
      backgroundColor: 'transparent',
      tooltip: {
        backgroundColor: '#131828',
        borderColor: '#232b41',
        textStyle: { color: '#e2e8f0' },
        formatter: (p: unknown) => {
          const d = (p as { data: [string, number] }).data;
          return `${d[0]}<br/><b>${fmtTokens(d[1])}</b> tokens`;
        },
      },
      visualMap: {
        min: 0,
        max,
        show: false,
        inRange: { color: ['#1a2138', '#4c3a80', '#a78bfa', '#e9d5ff'] },
      },
      calendar: {
        range: [iso(start), iso(end)],
        cellSize: ['auto', 14],
        dayLabel: { color: '#8b95ab', firstDay: 1 },
        monthLabel: { color: '#8b95ab' },
        yearLabel: { show: false },
        itemStyle: { color: '#10152a', borderColor: '#0b0f19', borderWidth: 2 },
        splitLine: { show: false },
      },
      series: [
        {
          type: 'heatmap',
          coordinateSystem: 'calendar',
          data: heatmap.map((c) => [c.date, c.tokens]),
        },
      ],
    } satisfies EChartsOption;
  });

  const hourOption = $derived.by(() => {
    if (!hourly.length) return undefined;
    const byHour = new Map(hourly.map((h) => [h.hour, h.tokens]));
    const hours = Array.from({ length: 24 }, (_, i) => i);
    const night = hourly.filter((h) => h.hour < 6).reduce((a, h) => a + h.tokens, 0);
    const total = hourly.reduce((a, h) => a + h.tokens, 0);
    return { option: {
      backgroundColor: 'transparent',
      tooltip: {
        backgroundColor: '#131828',
        borderColor: '#232b41',
        textStyle: { color: '#e2e8f0' },
        valueFormatter: (v: unknown) => fmtTokens(Number(v)),
      },
      grid: { left: 8, right: 12, top: 16, bottom: 0, containLabel: true },
      xAxis: {
        type: 'category',
        data: hours.map((h) => String(h)),
        axisLine: { lineStyle: { color: '#232b41' } },
        axisLabel: { color: '#8b95ab' },
      },
      yAxis: {
        type: 'value',
        axisLabel: { color: '#8b95ab', formatter: (v: number) => fmtTokens(v) },
        splitLine: { lineStyle: { color: 'rgba(35,43,65,0.5)' } },
      },
      series: [
        {
          type: 'bar',
          data: hours.map((h) => ({
            value: byHour.get(h) ?? 0,
            itemStyle: { color: h < 6 ? '#f472b6' : h >= 22 ? '#f472b6' : '#a78bfa' },
          })),
        },
      ],
    } satisfies EChartsOption, nightShare: total > 0 ? Math.round((night / total) * 100) : 0 };
  });
</script>

<h1>Activity</h1>
<p class="sub">Your coding-agent habits: every day, every hour, every harness combined</p>

{#if error}
  <p class="error">{error}</p>
{:else}
  {#if overview}
    <div class="cards">
      <div class="card">
        <div class="label">Current streak</div>
        <div class="value">{overview.current_streak} days</div>
      </div>
      <div class="card">
        <div class="label">Longest streak</div>
        <div class="value">{overview.longest_streak} days</div>
      </div>
      <div class="card">
        <div class="label">Active days</div>
        <div class="value">{overview.active_days}</div>
      </div>
      <div class="card">
        <div class="label">Night-owl share</div>
        <div class="value">{hourOption ? hourOption.nightShare : 0}%</div>
        <div class="hint">tokens between 00–06 and 22+</div>
      </div>
    </div>
  {/if}

  <div class="panel">
    <h2>Last 365 days</h2>
    {#if heatOption}
      <Chart option={heatOption} height={190} />
    {:else}
      <div class="loading">no usage recorded yet</div>
    {/if}
  </div>

  <div class="panel">
    <h2>Hour of day (all time)</h2>
    {#if hourOption}
      <Chart option={hourOption.option} height={230} />
    {:else}
      <div class="loading">no usage recorded yet</div>
    {/if}
  </div>
{/if}
