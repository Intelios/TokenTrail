<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import type { EChartsOption } from 'echarts';
  import Chart from '$lib/Chart.svelte';
  import { api, type Big6Timeline, type DailyCacheRow, type DailyModelRow } from '$lib/api';
  import { fmtTokens, familyFor, modelColor, modelSwatch, MIX_COLORS } from '$lib/format';
  import { TOOLTIP, ANIM, AXIS_LABEL, AXIS_LINE, SPLIT_LINE, LEGEND_TEXT, stackedBand, dateTick } from '$lib/chartTheme';

  const RANGES: [number, string][] = [
    [30, '30D'],
    [90, '90D'],
    [365, '1 YEAR'],
    [3650, 'ALL'],
  ];

  let days = $state(365);
  let byModel = $state<DailyModelRow[]>([]);
  let cache = $state<DailyCacheRow[]>([]);
  let timeline = $state<Big6Timeline | null>(null);
  let error = $state('');

  async function loadCharts() {
    try {
      const [m, c] = await Promise.all([api.dailyByModel(days), api.dailyCache(days)]);
      byModel = m;
      cache = c;
      error = '';
    } catch (e) {
      error = String(e);
    }
  }

  // The podium is a lifetime view: fetched once, re-fetched only on sync —
  // the range pills zoom its axis instead of narrowing the data.
  async function loadBig6() {
    try {
      timeline = await api.big6Timeline();
    } catch (e) {
      error = String(e);
    }
  }

  $effect(() => {
    // reload the windowed charts when the range changes
    days;
    loadCharts();
  });

  onMount(() => {
    loadBig6();
    const h = () => {
      loadCharts();
      loadBig6();
    };
    window.addEventListener('tt-sync', h);
    return () => window.removeEventListener('tt-sync', h);
  });

  const modelOption = $derived.by(() => {
    if (!byModel.length) return undefined;
    const dates = [...new Set(byModel.map((r) => r.date))].sort();
    const totals = new Map<string, number>();
    for (const r of byModel) totals.set(r.model, (totals.get(r.model) ?? 0) + r.tokens);
    // stack in rank order so band colors match the Models page ranking
    const models = [...totals.entries()].sort((a, b) => b[1] - a[1]).map(([m]) => m);
    const map = new Map(byModel.map((r) => [`${r.date}|${r.model}`, r.tokens]));
    return {
      backgroundColor: 'transparent',
      ...ANIM,
      animationDelay: (idx: number) => Math.min(idx, 8) * 90,
      tooltip: {
        trigger: 'axis',
        ...TOOLTIP,
        confine: true,
        formatter: (params: unknown) => {
          type TipParam = { axisValue?: string; marker: string; seriesName: string; value?: number };
          const all = params as TipParam[];
          const title = `<b>${all[0]?.axisValue ?? ''}</b>`;
          const list = all
            .filter((p) => Number(p.value ?? 0) > 0)
            .sort((a, b) => Number(b.value) - Number(a.value));
          if (!list.length) return `${title}<br/>no usage`;
          return (
            title +
            '<br/>' +
            list
              .map((p) => `${p.marker}${p.seriesName}&nbsp;&nbsp;<b>${fmtTokens(Number(p.value))}</b>`)
              .join('<br/>')
          );
        },
      },
      legend: { textStyle: LEGEND_TEXT, top: 0, type: 'scroll', icon: 'rect', itemWidth: 10, itemHeight: 10 },
      grid: { left: 8, right: 12, top: 34, bottom: 0, containLabel: true },
      xAxis: {
        type: 'category',
        data: dates,
        axisLine: AXIS_LINE,
        axisLabel: { ...AXIS_LABEL, hideOverlap: true, formatter: dateTick },
        axisTick: { show: false },
      },
      yAxis: {
        type: 'value',
        axisLabel: { ...AXIS_LABEL, formatter: (v: number) => fmtTokens(v) },
        splitLine: SPLIT_LINE,
      },
      series: models.map((m, i) =>
        stackedBand(m, dates.map((d) => map.get(`${d}|${m}`) ?? 0), modelColor(m, i), {
          delay: Math.min(i, 8) * 90,
        }),
      ),
    } satisfies EChartsOption;
  });

  const cacheOption = $derived.by(() => {
    if (!cache.length) return undefined;
    const dates = cache.map((r) => r.date);
    const series = [
      { name: 'Cache read', color: MIX_COLORS[2], key: 'cache_read' as const },
      { name: 'Fresh input', color: MIX_COLORS[0], key: 'fresh_input' as const },
      { name: 'Cache write', color: MIX_COLORS[3], key: 'cache_write' as const },
    ];
    return {
      backgroundColor: 'transparent',
      ...ANIM,
      animationDelay: (idx: number) => idx * 90,
      tooltip: {
        trigger: 'axis',
        ...TOOLTIP,
        valueFormatter: (v: unknown) => fmtTokens(Number(v)),
      },
      legend: { textStyle: LEGEND_TEXT, top: 0, icon: 'rect', itemWidth: 10, itemHeight: 10 },
      grid: { left: 8, right: 12, top: 34, bottom: 0, containLabel: true },
      xAxis: {
        type: 'category',
        data: dates,
        axisLine: AXIS_LINE,
        axisLabel: { ...AXIS_LABEL, hideOverlap: true, formatter: dateTick },
        axisTick: { show: false },
      },
      yAxis: {
        type: 'value',
        axisLabel: { ...AXIS_LABEL, formatter: (v: number) => fmtTokens(v) },
        splitLine: SPLIT_LINE,
      },
      series: series.map((s, i) =>
        stackedBand(s.name, cache.map((r) => r[s.key]), s.color, { delay: i * 90 }),
      ),
    } satisfies EChartsOption;
  });

  const cacheShare = $derived.by(() => {
    const read = cache.reduce((a, r) => a + r.cache_read, 0);
    const fresh = cache.reduce((a, r) => a + r.fresh_input, 0);
    const write = cache.reduce((a, r) => a + r.cache_write, 0);
    const total = read + fresh + write;
    return total > 0 ? Math.round((read / total) * 100) : 0;
  });

  // ── Big 6 podium ────────────────────────────────────────────────
  // Seat runs are UTC calendar days, same convention as the leaderboard
  // feed and the per-model Big-6 card. The right edge is tomorrow's UTC
  // midnight so today always gets its full slice of the track.

  const utcDay = (d: string) => new Date(`${d}T00:00:00Z`).getTime();

  function fmtDay(d: string): string {
    return new Date(`${d}T00:00:00Z`).toLocaleDateString(undefined, {
      day: 'numeric',
      month: 'short',
      year: 'numeric',
      timeZone: 'UTC',
    });
  }

  const podiumWindow = $derived.by(() => {
    if (!timeline?.first_date) return null;
    const now = new Date();
    const end = Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate() + 1);
    const first = utcDay(timeline.first_date);
    // ALL shows the whole history; the other pills zoom to the last N days,
    // clamped so the window never starts before the first recorded day.
    const daysMs = Math.min(days, 3650) * 86400000;
    const start = days >= 3650 ? first : Math.max(first, end - daysMs);
    return { start, end, zoomed: start > first };
  });

  // Fallback palette index per model: order of first seat appearance, so the
  // unbranded-model colors stay stable across renders and range switches.
  const modelIdx = $derived.by(() => {
    const m = new Map<string, number>();
    if (timeline) {
      for (const seat of timeline.seats) {
        for (const run of seat.runs) {
          if (!m.has(run.model)) m.set(run.model, m.size);
        }
      }
    }
    return m;
  });

  const podiumSeats = $derived.by(() => {
    const w = podiumWindow;
    if (!timeline || !w) return [];
    const pct = (t: number) => Math.max(0, Math.min(100, ((t - w.start) / (w.end - w.start)) * 100));
    return timeline.seats.map((seat) => ({
      rank: seat.rank,
      runs: seat.runs.flatMap((run, i) => {
        const l = pct(utcDay(run.entered));
        const r = pct(run.exited ? utcDay(run.exited) : w.end);
        if (r <= l) return [];
        const next = run.exited ? (seat.runs[i + 1]?.model ?? null) : null;
        return [
          {
            model: run.model,
            rank: seat.rank,
            entered: run.entered,
            exited: run.exited,
            days: run.days,
            next,
            // A boundary worth flagging when the seat stays in the family —
            // same-colour blocks would otherwise read as one unbroken run.
            seam: next !== null && familyFor(next) === familyFor(run.model) ? 'same' : 'diff',
            l,
            w: r - l,
            swatch: modelSwatch(run.model, modelIdx.get(run.model) ?? 0),
          },
        ];
      }),
    }));
  });

  interface PodiumTick {
    label: string;
    pct: number;
  }

  const podiumTicks = $derived.by(() => {
    const w = podiumWindow;
    if (!w) return [] as PodiumTick[];
    const winDays = Math.max(1, Math.round((w.end - w.start) / 86400000));
    const out: PodiumTick[] = [];
    const pct = (t: number) => ((t - w.start) / (w.end - w.start)) * 100;
    // density follows the zoom: days when close in, weeks a bit further out,
    // otherwise every Nth month to keep ~14 labels on the widest span
    const stepMs = winDays <= 21 ? 86400000 : winDays <= 120 ? 7 * 86400000 : 0;
    if (stepMs) {
      const s = new Date(w.start);
      let t = Date.UTC(s.getUTCFullYear(), s.getUTCMonth(), s.getUTCDate());
      while (t < w.start) t += stepMs;
      for (; t < w.end; t += stepMs) {
        const iso = new Date(t).toISOString().slice(0, 10);
        out.push({ label: stepMs === 86400000 ? dateTick(iso) : `w/c ${dateTick(iso)}`, pct: pct(t) });
      }
      return out;
    }
    const monthMs = 30.44 * 86400000;
    const step = Math.max(1, Math.ceil(winDays / monthMs / 14));
    const s = new Date(w.start);
    let y = s.getUTCFullYear();
    let mo = s.getUTCMonth() + 1;
    while (Date.UTC(y, mo - 1, 1) < w.start) {
      mo++;
      if (mo > 12) {
        mo = 1;
        y++;
      }
    }
    for (let i = 0; Date.UTC(y, mo - 1, 1) < w.end; i++) {
      const t = Date.UTC(y, mo - 1, 1);
      if (i % step === 0) {
        const label = `${new Date(t).toLocaleDateString('en-US', { month: 'short', timeZone: 'UTC' }).toUpperCase()} '${String(y).slice(2)}`;
        out.push({ label, pct: pct(t) });
      }
      mo++;
      if (mo > 12) {
        mo = 1;
        y++;
      }
    }
    return out;
  });

  const podiumStats = $derived.by(() => {
    if (!timeline) return null;
    // Read as takeovers, not raw holder transitions: the churny lower seats
    // swap for a day at a time, so only runs of 3+ days (and current
    // holders, however young) count toward the headline figure.
    const holders = timeline.seats[0]?.runs.length ?? 0;
    const takeovers = timeline.seats.reduce(
      (a, s) => a + s.runs.filter((r) => r.exited === null || r.days >= 3).length,
      0,
    );
    const w = podiumWindow;
    const note = w?.zoomed
      ? days === 365
        ? 'last year of all time'
        : `last ${days}d of all time`
      : null;
    return { holders, takeovers, note };
  });

  let tip = $state<{ x: number; y: number; html: string } | null>(null);

  function showTip(run: (typeof podiumSeats)[number]['runs'][number], e: MouseEvent) {
    const seat = `#${run.rank} seat · ${run.model}`;
    const range = `${fmtDay(run.entered)} → ${run.exited ? fmtDay(run.exited) : 'now'}`;
    const held = run.exited ? `held ${run.days}d` : run.days === 0 ? 'entered today' : `held ${run.days}d so far`;
    const next = run.next ? ` · taken by ${run.next}` : '';
    tip = {
      x: Math.min(e.clientX + 14, window.innerWidth - 280),
      y: e.clientY - 10,
      html: `<b>${seat}</b><br/>${range}<br/>${held}${next}`,
    };
  }

  function moveTip(e: MouseEvent) {
    if (tip) tip = { ...tip, x: Math.min(e.clientX + 14, window.innerWidth - 280), y: e.clientY - 10 };
  }

  function showTipAt(run: (typeof podiumSeats)[number]['runs'][number], el: HTMLElement) {
    const r = el.getBoundingClientRect();
    showTip(run, { clientX: r.left + r.width / 2, clientY: r.top - 12 } as MouseEvent);
  }

  function openModel(model: string) {
    goto('/models/' + encodeURIComponent(model));
  }
</script>

<div class="tframe">
  <div class="thd">
    <div class="up">
      <h1>Trends</h1>
      <div class="sub">Which models you actually use, and how much context gets served from cache</div>
    </div>
    <div class="pills up">
      {#each RANGES as [d, label]}
        <button class="pill" class:on={days === d} onclick={() => (days = d)}>{label}</button>
      {/each}
    </div>
  </div>

  {#if error}
    <div class="errpad"><p class="error">{error}</p></div>
  {:else}
    <div class="tpanel up">
      <h2>
        All-time Big 6 — seats
        {#if podiumStats}
          <span class="rt">
            <b class="delta">#1 seat · {podiumStats.holders} holder{podiumStats.holders === 1 ? '' : 's'}</b>
            <span class="wn">{podiumStats.takeovers} takeovers ≥3d</span>
            {#if podiumStats.note}<span class="wn">{podiumStats.note}</span>{/if}
          </span>
        {/if}
      </h2>
      {#if !timeline || !podiumSeats.some((s) => s.runs.length)}
        <div class="loading">no usage recorded yet</div>
      {:else}
        <div class="podium">
          <div class="gl">
            {#each podiumTicks as t}
              <i style="left:{t.pct}%"></i>
            {/each}
            <b class="now"></b>
          </div>
          {#each podiumSeats as seat}
            <div class="seat">
              <div class="rk">#{seat.rank}</div>
              <div class="track">
                {#each seat.runs as run}
                  <button
                    class="seg"
                    style="left:{run.l}%;width:{run.w}%;background:{run.swatch}"
                    onmouseenter={(e) => showTip(run, e)}
                    onmousemove={moveTip}
                    onmouseleave={() => (tip = null)}
                    onfocus={(e) => showTipAt(run, e.currentTarget)}
                    onblur={() => (tip = null)}
                    onclick={() => openModel(run.model)}
                  >
                    {#if run.w >= 10}<span class="nm">{run.model}</span>{:else if run.w >= 3.5}<span class="dur">{run.days}d</span>{/if}
                    {#if run.w >= 10}<span class="dur">{run.days}d</span>{/if}
                  </button>
                {/each}
                {#each seat.runs as run}
                  {#if run.next && run.w >= 0.5}
                    <i class="seam" style="left:{run.l + run.w}%"></i>
                    {#if run.seam === 'same'}
                      <svg class="bead" style="left:{run.l + run.w}%" viewBox="0 0 9 9" aria-hidden="true">
                        <polygon points="4.5,0.7 8.3,4.5 4.5,8.3 0.7,4.5" fill="var(--bone)" stroke="var(--ink)" stroke-width="1.2" />
                      </svg>
                    {/if}
                  {/if}
                {/each}
              </div>
              {#if seat.runs.some((r) => r.exited === null)}<span class="inflag">IN</span>{/if}
            </div>
          {/each}
          <div class="axis">
            {#each podiumTicks as t}
              {#if t.pct <= 94}<span style="left:{t.pct}%">{t.label}</span>{/if}
            {/each}
            <span class="tlab">TODAY</span>
          </div>
        </div>
      {/if}
    </div>

    <div class="tpanel up" style="animation-delay:60ms">
      <h2>Model share over time</h2>
      {#if modelOption}
        <Chart option={modelOption} height={320} />
      {:else}
        <div class="loading">no usage recorded yet</div>
      {/if}
    </div>

    <div class="tpanel up" style="animation-delay:120ms">
      <h2>
        Context: cache vs fresh<span class="rt"><b class="delta">{cacheShare}% served from cache read</b></span>
      </h2>
      {#if cacheOption}
        <Chart option={cacheOption} height={280} />
      {:else}
        <div class="loading">no usage recorded yet</div>
      {/if}
    </div>
  {/if}
</div>

{#if tip}
  <div class="tip" style="left:{tip.x}px;top:{tip.y}px">{@html tip.html}</div>
{/if}

<style>
  .tframe {
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

  .tpanel {
    background: var(--bone);
    border-bottom: 2px solid var(--ink);
    padding: 16px clamp(22px, 1.8vw, 40px) 18px;
  }
  .tpanel:last-child {
    border-bottom: none;
  }
  .tpanel h2 {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    font: 600 11px/1 var(--font-ui);
    letter-spacing: 1.6px;
    text-transform: uppercase;
    margin: 0 0 12px;
  }

  .rt {
    display: flex;
    gap: 14px;
    align-items: baseline;
  }
  .delta {
    font: 500 10px/1 var(--font-mono);
    letter-spacing: 1px;
    color: var(--org);
  }
  .wn {
    font: 500 9.5px/1 var(--font-mono);
    letter-spacing: 1px;
    color: var(--dim);
  }

  /* ── Big 6 podium: six seat lanes over a shared UTC axis ─────── */

  .podium {
    position: relative;
    /* Right gutter outside the bars' axis: the IN flag parks there so it
       never covers the current holder's block. */
    --flagw: 28px;
  }

  .gl {
    position: absolute;
    top: 0;
    bottom: 18px;
    left: 56px;
    right: var(--flagw);
    pointer-events: none;
  }
  .gl i {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 1px;
    background: var(--hair);
  }
  .gl .now {
    position: absolute;
    top: 0;
    bottom: 0;
    right: 0;
    width: 2px;
    background: var(--org);
  }

  .seat {
    display: flex;
    align-items: stretch;
    height: 32px;
    border-bottom: 1px solid var(--hair);
    position: relative;
    padding-right: var(--flagw);
  }
  .rk {
    width: 48px;
    flex: none;
    font: 400 15px/32px var(--font-disp);
    opacity: 0.65;
  }
  .track {
    position: relative;
    flex: 1;
    margin-left: 8px;
    overflow: hidden;
  }

  .seg {
    position: absolute;
    top: 5px;
    height: 22px;
    min-width: 2px;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 7px;
    border: none;
    border-radius: 0;
    overflow: hidden;
    white-space: nowrap;
    cursor: pointer;
    font-family: var(--font-ui);
    text-transform: none;
    letter-spacing: 0;
  }
  .seg:hover {
    filter: brightness(1.12);
    outline: 1.5px solid var(--ink);
    z-index: 2;
  }
  .seg .nm {
    font: 600 10px/1 var(--font-ui);
    color: #fff;
    letter-spacing: 0.2px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .seg .dur {
    font: 600 8.5px/1 var(--font-mono);
    color: rgba(255, 255, 255, 0.85);
    flex: none;
    margin-left: auto;
  }

  /* Handover seams: every seat change gets a notch; a handover that keeps
     the seat in the same family gets a pennant over the boundary, since the
     two blocks share a colour and would read as one run. */
  .seam {
    position: absolute;
    top: 5px;
    width: 1px;
    height: 22px;
    transform: translateX(-0.5px);
    background: var(--bone);
    pointer-events: none;
    z-index: 2;
  }
  .seam::before,
  .seam::after {
    content: '';
    position: absolute;
    left: -2px;
    border-left: 2.5px solid transparent;
    border-right: 2.5px solid transparent;
  }
  .seam::before {
    top: 0;
    border-top: 3.5px solid var(--ink);
  }
  .seam::after {
    bottom: 0;
    border-bottom: 3.5px solid var(--ink);
  }
  /* Same-family handovers: a small bone "joint" bead on the seam — the two
     blocks share a colour, so the silhouette alone would read as one run. */
  .bead {
    position: absolute;
    top: 16px;
    width: 9px;
    height: 9px;
    transform: translate(-50%, -50%);
    pointer-events: none;
    z-index: 3;
  }

  .inflag {
    position: absolute;
    right: 0;
    top: 5px;
    height: 22px;
    display: flex;
    align-items: center;
    padding: 0 4px;
    font: 700 7.5px/1 var(--font-mono);
    letter-spacing: 0.5px;
    background: var(--acd);
    color: var(--ink);
    z-index: 3;
    pointer-events: none;
  }

  .axis {
    position: relative;
    height: 18px;
    margin-left: 56px;
    margin-right: var(--flagw);
  }
  .axis span {
    position: absolute;
    top: 4px;
    transform: translateX(-50%);
    font: 500 9px/1 var(--font-mono);
    color: var(--dim);
    letter-spacing: 0.4px;
    white-space: nowrap;
  }
  .axis .tlab {
    left: auto;
    right: 0;
    transform: none;
    color: var(--org);
    font-weight: 600;
    letter-spacing: 0.8px;
  }

  .tip {
    position: fixed;
    z-index: 80;
    background: var(--ink);
    color: var(--bone);
    border: 2px solid var(--ink);
    padding: 8px 12px;
    font: 500 11px/1.55 var(--font-mono);
    pointer-events: none;
    white-space: nowrap;
  }
</style>
