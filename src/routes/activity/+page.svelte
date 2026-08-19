<script lang="ts">
  import { onMount } from 'svelte';
  import type { EChartsOption } from 'echarts';
  import Chart from '$lib/Chart.svelte';
  import AnimatedNumber from '$lib/AnimatedNumber.svelte';
  import { api, type HeatmapCell, type HourRow, type Overview } from '$lib/api';
  import { fmtTokens } from '$lib/format';
  import { TOOLTIP, ANIM, AXIS_LABEL, AXIS_LINE, SPLIT_LINE, MONO, DIM } from '$lib/chartTheme';

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
      ...ANIM,
      tooltip: {
        ...TOOLTIP,
        formatter: (p: any) => {
          const d = p.data as [string, number];
          return `${d[0]}<br/><b>${fmtTokens(d[1])}</b> tokens`;
        },
      },
      visualMap: {
        min: 0,
        max,
        show: false,
        // Marathon ramp: bone ground through ink steps to orange peaks
        inRange: { color: ['#e8e4d9', 'rgba(13,13,11,0.28)', 'rgba(13,13,11,0.6)', '#0d0d0b', '#ff4d00'] },
      },
      calendar: {
        range: [iso(start), iso(end)],
        cellSize: ['auto' as const, 14],
        dayLabel: { color: DIM, fontFamily: MONO, fontSize: 9 },
        monthLabel: { color: DIM, fontFamily: MONO, fontSize: 9 },
        yearLabel: { show: false },
        itemStyle: { color: 'rgba(13,13,11,0.06)', borderColor: '#e8e4d9', borderWidth: 2 },
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
    return {
      backgroundColor: 'transparent',
      ...ANIM,
      tooltip: {
        trigger: 'axis',
        ...TOOLTIP,
        valueFormatter: (v: unknown) => fmtTokens(Number(v)),
      },
      grid: { left: 8, right: 8, top: 20, bottom: 0, containLabel: true },
      xAxis: {
        type: 'category',
        data: hours.map((h) => String(h).padStart(2, '0')),
        axisLine: AXIS_LINE,
        axisLabel: AXIS_LABEL,
        axisTick: { show: false },
      },
      yAxis: {
        type: 'value',
        axisLabel: { ...AXIS_LABEL, formatter: (v: number) => fmtTokens(v) },
        splitLine: SPLIT_LINE,
      },
      series: [
        {
          type: 'bar',
          data: hours.map((h) => ({
            value: byHour.get(h) ?? 0,
            itemStyle: { color: h < 6 || h >= 22 ? '#ff1f6f' : '#0d0d0b' },
          })),
          animationDelay: (idx: number) => idx * 25,
        },
      ],
    } satisfies EChartsOption;
  });

  const nightShare = $derived.by(() => {
    const night = hourly.filter((h) => h.hour < 6 || h.hour >= 22).reduce((a, h) => a + h.tokens, 0);
    const total = hourly.reduce((a, h) => a + h.tokens, 0);
    return total > 0 ? Math.round((night / total) * 100) : 0;
  });
</script>

<h1 class="up">Activity</h1>
<p class="sub">Your coding-agent habits: every day, every hour, every harness combined</p>

{#if error}
  <p class="error">{error}</p>
{:else}
  {#if overview}
    <div class="cards">
      <div class="card up">
        <div class="label">Current streak</div>
        <div class="value"><AnimatedNumber value={overview.current_streak} format={(n) => `${Math.round(n)}d`} /></div>
        <div class="hint">days in a row</div>
      </div>
      <div class="card up" style="animation-delay:60ms">
        <div class="label">Longest streak</div>
        <div class="value"><AnimatedNumber value={overview.longest_streak} format={(n) => `${Math.round(n)}d`} /></div>
        <div class="hint">personal record</div>
      </div>
      <div class="card up" style="animation-delay:120ms">
        <div class="label">Active days</div>
        <div class="value"><AnimatedNumber value={overview.active_days} /></div>
        <div class="hint">since {overview.first_ts ? new Date(overview.first_ts).getFullYear() : '—'}</div>
      </div>
      <div class="card up" style="animation-delay:180ms">
        <div class="label">Night-owl share</div>
        <div class="value"><AnimatedNumber value={nightShare} format={(n) => `${Math.round(n)}%`} /></div>
        <div class="hint">tokens 22:00–06:00</div>
      </div>
    </div>
  {/if}

  <div class="panel up">
    <h2>Last 365 days<span class="fig">FIG.14</span></h2>
    {#if heatOption}
      <Chart option={heatOption} height={190} />
    {:else}
      <div class="loading">no usage recorded yet</div>
    {/if}
  </div>

  <div class="panel up" style="animation-delay:100ms">
    <h2>Hour of day — all time<span class="fig">FIG.15</span></h2>
    {#if hourOption}
      <Chart option={hourOption} height={230} />
    {:else}
      <div class="loading">no usage recorded yet</div>
    {/if}
  </div>
{/if}
