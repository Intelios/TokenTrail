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
            itemStyle: { color: h < 6 || h >= 22 ? '#3d8eff' : '#ff4d00' },
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

<div class="aframe">
  <div class="thd">
    <div class="up">
      <h1>Activity</h1>
      <div class="sub">Your coding-agent habits: every day, every hour, every harness combined</div>
    </div>
  </div>

  {#if error}
    <div class="errpad"><p class="error">{error}</p></div>
  {:else}
    {#if overview}
      <div class="mcards">
        <div class="mc up">
          <div class="k">Current streak</div>
          <div class="v"><AnimatedNumber value={overview.current_streak} format={(n) => `${Math.round(n)}d`} /></div>
          <div class="h">days in a row</div>
        </div>
        <div class="mc up" style="animation-delay:60ms">
          <div class="k">Longest streak</div>
          <div class="v"><AnimatedNumber value={overview.longest_streak} format={(n) => `${Math.round(n)}d`} /></div>
          <div class="h">personal record</div>
        </div>
        <div class="mc up" style="animation-delay:120ms">
          <div class="k">Active days</div>
          <div class="v"><AnimatedNumber value={overview.active_days} /></div>
          <div class="h">since {overview.first_ts ? new Date(overview.first_ts).getFullYear() : '—'}</div>
        </div>
        <div class="mc up" style="animation-delay:180ms">
          <div class="k">Night-owl share</div>
          <div class="v"><AnimatedNumber value={nightShare} format={(n) => `${Math.round(n)}%`} /></div>
          <div class="h">tokens 22:00–06:00</div>
        </div>
      </div>
    {/if}

    <div class="apanel up">
      <h2>Last 365 days</h2>
      {#if heatOption}
        <Chart option={heatOption} height={190} />
      {:else}
        <div class="loading">no usage recorded yet</div>
      {/if}
    </div>

    <div class="apanel up" style="animation-delay:100ms">
      <h2>
        Hour of day — all time
        <span class="legend-row">
          <span class="leg-item"><i style="background:#ff4d00"></i>Day (06:00–22:00)</span>
          <span class="leg-item"><i style="background:#3d8eff"></i>Night (22:00–06:00)</span>
        </span>
      </h2>
      {#if hourOption}
        <Chart option={hourOption} height={230} />
      {:else}
        <div class="loading">no usage recorded yet</div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .aframe {
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
  .thd .sub { font: 400 11px/1 var(--font-mono); opacity: 0.55; margin-top: 6px; letter-spacing: 0.6px; margin-bottom: 0; }

  .errpad { padding: 20px 22px; }

  .mcards { display: grid; grid-template-columns: repeat(4, 1fr); border-bottom: 2px solid var(--ink); }
  .mc { padding: clamp(12px, 1.2vh, 22px) clamp(16px, 1.2vw, 26px); border-right: 2px solid var(--ink); min-width: 0; }
  .mc:last-child { border-right: none; }
  .mc .k { font: 600 clamp(9px, 0.7vw, 12px)/1 var(--font-ui); letter-spacing: 1.3px; text-transform: uppercase; opacity: 0.55; }
  .mc .v { font: 400 clamp(26px, 2.1vw, 44px)/1 var(--font-disp); margin-top: 8px; letter-spacing: -1px; font-variant-numeric: tabular-nums; }
  .mc .h { font: 400 clamp(10px, 0.75vw, 12px)/1.3 var(--font-mono); margin-top: 7px; opacity: 0.48; }

  .apanel {
    background: var(--bone);
    border-bottom: 2px solid var(--ink);
    padding: 16px clamp(22px, 1.8vw, 40px) 18px;
  }
  .apanel:last-child {
    border-bottom: none;
  }
  .apanel h2 {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    font: 600 11px/1 var(--font-ui);
    letter-spacing: 1.6px;
    text-transform: uppercase;
    margin: 0 0 12px;
  }

  .legend-row {
    display: flex;
    gap: 14px;
    align-items: center;
  }
  .leg-item {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font: 500 10px/1 var(--font-mono);
    letter-spacing: 0.8px;
    text-transform: uppercase;
    opacity: 0.75;
  }
  .leg-item i {
    width: 8px;
    height: 8px;
    display: inline-block;
    flex: none;
  }

  @media (max-width: 900px) {
    .mcards { grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); }
    .mc { border-bottom: 2px solid var(--ink); }
  }
</style>
