<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/stores';
  import type { EChartsOption } from 'echarts';
  import AnimatedNumber from '$lib/AnimatedNumber.svelte';
  import Chart from '$lib/Chart.svelte';
  import {
    api,
    type CompareModel,
    type ComparisonDailyRow,
    type ModelComparison,
    type ModelStatsRow,
    type ProjectRow,
  } from '$lib/api';
  import { dailyColumns, dailyRange, type Column } from '$lib/dailyColumns';
  import {
    basename,
    compareColors,
    fmtCost,
    fmtCostExact,
    fmtDate,
    fmtDuration,
    fmtTokens,
    fmtTokensExact,
    MIX_COLORS,
    sourceLabel,
    sourceSwatch,
  } from '$lib/format';
  import { readPref, writePref } from '$lib/prefs';
  import { ANIM, AXIS_LABEL, AXIS_LINE, DIM, MONO, SPLIT_LINE, TOOLTIP } from '$lib/chartTheme';

  const RANGES: [number, string][] = [
    [7, '7D'],
    [30, '30D'],
    [90, '90D'],
    [3650, 'ALL'],
  ];
  const ANCHORS = [1_000_000, 10_000_000, 50_000_000, 100_000_000];
  const RANGE_LABEL: Record<number, string> = { 7: '7 days', 30: '30 days', 90: '90 days', 3650: 'all time' };
  const PREF_DAYS = 'tt.compare.days';
  const PREF_ANCHOR = 'tt.compare.anchorTokens';
  const PREF_RACE = 'tt.compare.raceMode';
  const PREF_PAIR = 'tt.compare.pair';

  type RaceMode = 'daily' | 'cumulative';
  type Side = 'a' | 'b';

  let catalog = $state<ModelStatsRow[]>([]);
  let catalogReady = $state(false);
  let result = $state<ModelComparison | null>(null);
  let days = $state(readPref(PREF_DAYS, 90, (v) => RANGES.some(([d]) => d === v)));
  let anchorTokens = $state(readPref(PREF_ANCHOR, 10_000_000, (v) => ANCHORS.includes(v as number)));
  let raceMode = $state<RaceMode>(readPref(PREF_RACE, 'daily', (v) => v === 'daily' || v === 'cumulative'));
  let loading = $state(true);
  let busy = $state(false);
  let error = $state('');
  let notice = $state('');
  let sequence = 0;

  const urlA = $derived($page.url.searchParams.get('a') ?? '');
  const urlB = $derived($page.url.searchParams.get('b') ?? '');
  const colors = $derived(result ? compareColors(result.a.model, result.b.model) : ['#ff4d00', '#00c2c2']);

  function validPair(a: string, b: string, rows: ModelStatsRow[]): boolean {
    return a !== b && rows.some((m) => m.model === a) && rows.some((m) => m.model === b);
  }

  async function navigatePair(a: string, b: string, replace = false) {
    const query = new URLSearchParams($page.url.search);
    query.set('a', a);
    query.set('b', b);
    await goto(`${$page.url.pathname}?${query}`, { replaceState: replace, noScroll: true, keepFocus: true });
  }

  async function loadCatalog() {
    try {
      const rows = (await api.modelStats(3650)).filter((m) => m.model !== 'unknown');
      catalog = rows;
      if (rows.length < 2) {
        catalogReady = true;
        loading = false;
        return;
      }

      let a = urlA;
      let b = urlB;
      if (!validPair(a, b, rows)) {
        const remembered = readPref(PREF_PAIR, { a: '', b: '' }, (v): v is { a: string; b: string } => {
          if (typeof v !== 'object' || v === null) return false;
          const pair = v as Record<string, unknown>;
          return typeof pair.a === 'string' && typeof pair.b === 'string';
        });
        a = remembered.a;
        b = remembered.b;
      }
      if (!validPair(a, b, rows)) {
        a = rows[0].model;
        b = rows[1].model;
      }
      if (a !== urlA || b !== urlB) {
        notice = urlA || urlB ? 'That matchup is unavailable. Restored your latest valid pair.' : '';
        await navigatePair(a, b, true);
      }
      writePref(PREF_PAIR, { a, b });
      catalogReady = true;
      error = '';
    } catch (e) {
      error = String(e);
      catalogReady = true;
      loading = false;
    }
  }

  async function loadComparison(modelA: string, modelB: string, range: number, anchor: number, quiet = false) {
    const request = ++sequence;
    const samePair = result?.a.model === modelA && result?.b.model === modelB;
    busy = true;
    if (!samePair) result = null;
    loading = !result;
    if (!quiet) error = '';
    try {
      const next = await api.modelComparison(modelA, modelB, range, anchor);
      if (request !== sequence) return;
      result = next;
      writePref(PREF_PAIR, { a: modelA, b: modelB });
      error = '';
    } catch (e) {
      if (request === sequence) error = String(e);
    } finally {
      if (request === sequence) {
        loading = false;
        busy = false;
      }
    }
  }

  async function retry() {
    await loadCatalog();
    if (urlA && urlB && urlA !== urlB) await loadComparison(urlA, urlB, days, anchorTokens);
  }

  function changeSide(side: Side, event: Event) {
    const value = (event.currentTarget as HTMLSelectElement).value;
    if (!value) return;
    const a = side === 'a' ? value : urlA;
    const b = side === 'b' ? value : urlB;
    if (a === b) return;
    notice = '';
    void navigatePair(a, b);
  }

  function swap() {
    if (!urlA || !urlB || urlA === urlB) return;
    notice = '';
    void navigatePair(urlB, urlA);
  }

  $effect(() => {
    const a = urlA;
    const b = urlB;
    const range = days;
    const anchor = anchorTokens;
    if (!catalogReady || !a || !b || a === b) return;
    untrack(() => void loadComparison(a, b, range, anchor));
  });

  $effect(() => {
    writePref(PREF_DAYS, days);
    writePref(PREF_ANCHOR, anchorTokens);
    writePref(PREF_RACE, raceMode);
  });

  onMount(() => {
    void loadCatalog();
    const sync = async () => {
      await loadCatalog();
      if (urlA && urlB && urlA !== urlB) await loadComparison(urlA, urlB, days, anchorTokens, true);
    };
    window.addEventListener('tt-sync', sync);
    return () => {
      sequence++;
      window.removeEventListener('tt-sync', sync);
    };
  });

  function percent(value: number, total: number): number | null {
    return total > 0 ? (value / total) * 100 : null;
  }

  function fmtPct(value: number | null): string {
    return value === null ? '—' : `${value.toFixed(1)}%`;
  }

  function fmtRate(value: number | null): string {
    return value === null ? '—' : `${fmtCost(value)} / M tok`;
  }

  function fmtDay(date: string | null): string {
    if (!date) return '—';
    return new Date(`${date}T00:00:00Z`).toLocaleDateString(undefined, {
      day: 'numeric',
      month: 'short',
      year: 'numeric',
      timeZone: 'UTC',
    });
  }

  function fmtHour(hour: number | null): string {
    return hour === null ? '—' : `${String(hour).padStart(2, '0')}:00 UTC`;
  }

  function delta(a: number | null, b: number | null, lower = false): string {
    if (a === null || b === null) return '—';
    if (a === b) return 'Tie';
    const aLeads = lower ? a < b : a > b;
    const base = aLeads ? b : a;
    const difference = Math.abs(a - b);
    if (base === 0) return `${aLeads ? 'A' : 'B'} ${lower ? 'lower' : 'leads'}`;
    const amount = (difference / Math.abs(base)) * 100;
    return `${aLeads ? 'A' : 'B'} ${lower ? '−' : '+'}${amount >= 100 ? Math.round(amount) : amount.toFixed(1)}%`;
  }

  function costPerCall(model: CompareModel): number | null {
    return model.priced_cost_usd !== null && model.priced_events > 0
      ? model.priced_cost_usd / model.priced_events
      : null;
  }

  function coverage(model: CompareModel): number | null {
    return percent(model.priced_tokens, model.measured_tokens);
  }

  function mix(model: CompareModel) {
    const total = model.input_tokens + model.output_tokens + model.cache_read_tokens + model.cache_write_tokens;
    return [
      { name: 'Fresh input', tokens: model.input_tokens, color: MIX_COLORS[0] },
      { name: 'Output', tokens: model.output_tokens, color: MIX_COLORS[1] },
      { name: 'Cache read', tokens: model.cache_read_tokens, color: MIX_COLORS[2] },
      { name: 'Cache write', tokens: model.cache_write_tokens, color: MIX_COLORS[3] },
    ].map((part) => ({ ...part, share: total > 0 ? (part.tokens / total) * 100 : 0 }));
  }

  function projectUrl(project: string): string {
    return '/projects/' + encodeURIComponent(project);
  }

  function modelUrl(model: string): string {
    return '/models/' + encodeURIComponent(model);
  }

  function sharedProjects(models: CompareModel[]): Set<string> {
    const [a, b] = models;
    const other = new Set(b.by_project.map((p) => p.project));
    return new Set(a.by_project.map((p) => p.project).filter((p) => p !== 'unknown' && other.has(p)));
  }

  function sharedSources(models: CompareModel[]): Set<string> {
    const [a, b] = models;
    const other = new Set(b.by_source.map((s) => s.source));
    return new Set(a.by_source.map((s) => s.source).filter((s) => other.has(s)));
  }

  function projectShare(project: ProjectRow, model: CompareModel): string {
    return fmtPct(percent(project.tokens, model.tokens));
  }

  function speedDelta(a: number | null, b: number | null, ac: number, bc: number, lower = false): string {
    if (a === null || b === null) return 'Not comparable';
    if (ac < 20 || bc < 20) return 'Need 20 eligible calls each';
    return delta(a, b, lower);
  }

  const hero = $derived.by(() => {
    if (!result?.cost_comparison) return null;
    const cost = result.cost_comparison;
    const rateA = result.a.cost_per_mtok ?? 0;
    const rateB = result.b.cost_per_mtok ?? 0;
    const paid = rateA > 0 && rateB > 0 && cost.b_equivalent_tokens !== null;
    const bothFree = rateA === 0 && rateB === 0;
    return {
      paid,
      bothFree,
      aTokens: cost.anchor_tokens,
      bTokens: paid ? cost.b_equivalent_tokens! : cost.anchor_tokens,
      aCost: cost.a_anchor_cost_usd,
      bCost: paid ? cost.a_anchor_cost_usd : cost.b_anchor_cost_usd,
      title: paid
        ? 'Equal-cost mileage'
        : bothFree
          ? 'Both models are free at bundled list prices'
          : rateA === 0
            ? `${result.a.model} is free at bundled list prices`
            : `${result.b.model} is free at bundled list prices`,
    };
  });

  const dailyRows = $derived.by(() => {
    if (!result) return [] as ComparisonDailyRow[];
    return result.daily;
  });
  const raceColumns = $derived.by(() => {
    if (!result) return [] as Column[];
    return dailyColumns(
      dailyRows.flatMap((d) => [
        { date: d.date, source: 'a', tokens: d.a_tokens, cost_usd: null },
        { date: d.date, source: 'b', tokens: d.b_tokens, cost_usd: null },
      ]),
      ['a', 'b'],
    );
  });
  const raceRange = $derived(dailyRange(raceColumns));

  const raceOption = $derived.by((): EChartsOption | undefined => {
    if (!result || !raceColumns.length) return undefined;
    const a = result.a;
    const b = result.b;
    const cumulative = raceMode === 'cumulative';
    const cats = raceColumns.map((c, i) => (c.kind === 'day' ? new Date(c.start).toISOString().slice(0, 10) : `quiet-${i}`));
    const labels = new Map(
      raceColumns.map((c, i) => [
        cats[i],
        c.kind === 'day'
          ? new Date(c.start).toLocaleDateString('en-US', { month: 'short', day: 'numeric', timeZone: 'UTC' }).toUpperCase()
          : '',
      ]),
    );
    let runningA = 0;
    let runningB = 0;
    const seriesA: Array<number | null> = [];
    const seriesB: Array<number | null> = [];
    for (const col of raceColumns) {
      if (col.kind === 'day') {
        runningA += col.values[0];
        runningB += col.values[1];
        seriesA.push(cumulative ? runningA : col.values[0]);
        seriesB.push(cumulative ? runningB : col.values[1]);
      } else {
        seriesA.push(cumulative ? runningA : null);
        seriesB.push(cumulative ? runningB : null);
      }
    }
    const hasQuiet = raceColumns.some((c) => c.kind === 'quiet');
    return {
      backgroundColor: 'transparent',
      ...ANIM,
      tooltip: {
        trigger: 'axis',
        ...TOOLTIP,
        confine: true,
        formatter: (params: unknown) => {
          const all = params as Array<{ dataIndex: number; seriesName: string; value?: number | null; marker: string }>;
          const col = raceColumns[all[0]?.dataIndex ?? -1];
          if (!col) return '';
          if (col.kind === 'quiet') {
            return `<b>${col.days} quiet days</b><br/>${cumulative ? 'cumulative totals unchanged' : 'no usage'}`;
          }
          const title = `<b>${fmtDay(new Date(col.start).toISOString().slice(0, 10))}</b>`;
          const rows = all
            .filter((p) => p.seriesName === a.model || p.seriesName === b.model)
            .map((p) => `${p.marker}${p.seriesName}&nbsp;&nbsp;<b>${fmtTokens(Number(p.value ?? 0))}</b>`);
          return `${title}<br/>${rows.join('<br/>')}`;
        },
      },
      grid: { left: 8, right: 14, top: 18, bottom: 0, containLabel: true },
      xAxis: {
        type: 'category',
        data: cats,
        axisLine: AXIS_LINE,
        axisTick: { show: false },
        axisLabel: { ...AXIS_LABEL, hideOverlap: true, formatter: (v: string) => labels.get(v) ?? '' },
      },
      yAxis: [
        { type: 'value', min: 0, axisLabel: { ...AXIS_LABEL, formatter: (v: number) => fmtTokens(v) }, splitLine: SPLIT_LINE },
        { type: 'value', min: 0, max: 1, show: false },
      ],
      series: [
        {
          name: a.model,
          type: 'line',
          data: seriesA,
          symbol: 'none',
          connectNulls: false,
          lineStyle: { width: 2, color: colors[0], type: 'solid' },
          itemStyle: { color: colors[0] },
          emphasis: { focus: 'series' },
        },
        {
          name: b.model,
          type: 'line',
          data: seriesB,
          symbol: 'none',
          connectNulls: false,
          lineStyle: { width: 2, color: colors[1], type: 'dashed' },
          itemStyle: { color: colors[1] },
          emphasis: { focus: 'series' },
        },
        ...(hasQuiet
          ? [
              {
                name: 'quiet',
                type: 'bar' as const,
                yAxisIndex: 1,
                barGap: '-100%',
                z: 0,
                silent: true,
                itemStyle: { color: 'rgba(13,13,11,0.05)' },
                label: {
                  show: true,
                  position: 'inside' as const,
                  rotate: 90,
                  color: DIM,
                  fontFamily: MONO,
                  fontSize: 9,
                  formatter: (p: { dataIndex: number }) => {
                    const col = raceColumns[p.dataIndex];
                    return col?.kind === 'quiet' ? `${col.days} QUIET DAYS` : '';
                  },
                },
                data: raceColumns.map((c) => (c.kind === 'quiet' ? 1 : null)),
              },
            ]
          : []),
      ],
    } satisfies EChartsOption;
  });

  const hourOption = $derived.by((): EChartsOption | undefined => {
    if (!result || (!result.a.tokens && !result.b.tokens)) return undefined;
    const a = result.a;
    const b = result.b;
    const hours = Array.from({ length: 24 }, (_, i) => i);
    return {
      backgroundColor: 'transparent',
      ...ANIM,
      tooltip: {
        trigger: 'axis',
        ...TOOLTIP,
        confine: true,
        formatter: (params: unknown) => {
          const all = params as Array<{ axisValue: string; seriesName: string; value: number; marker: string }>;
          const hour = Number(all[0]?.axisValue ?? 0);
          const rows = all.map((p) => {
            const model = p.seriesName === a.model ? a : b;
            return `${p.marker}${p.seriesName}&nbsp;&nbsp;<b>${p.value.toFixed(1)}%</b> · ${fmtTokens(model.hourly[hour]?.tokens ?? 0)}`;
          });
          return `<b>${fmtHour(hour)}</b><br/>${rows.join('<br/>')}`;
        },
      },
      grid: { left: 8, right: 14, top: 18, bottom: 0, containLabel: true },
      xAxis: {
        type: 'category',
        data: hours.map((h) => String(h).padStart(2, '0')),
        axisLine: AXIS_LINE,
        axisLabel: AXIS_LABEL,
        axisTick: { show: false },
      },
      yAxis: {
        type: 'value',
        min: 0,
        axisLabel: { ...AXIS_LABEL, formatter: (v: number) => `${v}%` },
        splitLine: SPLIT_LINE,
      },
      series: [
        {
          name: a.model,
          type: 'line',
          data: a.hourly.map((h) => (a.tokens ? (h.tokens / a.tokens) * 100 : 0)),
          symbol: 'none',
          lineStyle: { width: 2, color: colors[0], type: 'solid' },
          itemStyle: { color: colors[0] },
        },
        {
          name: b.model,
          type: 'line',
          data: b.hourly.map((h) => (b.tokens ? (h.tokens / b.tokens) * 100 : 0)),
          symbol: 'none',
          lineStyle: { width: 2, color: colors[1], type: 'dashed' },
          itemStyle: { color: colors[1] },
        },
      ],
    } satisfies EChartsOption;
  });
</script>

<div class="cframe">
  <header class="thd">
    <div class="up">
      <h1>Compare</h1>
    </div>
    <div class="head-controls up">
      {#if busy}<span class="updating" aria-live="polite">updating…</span>{/if}
      <div class="pills" aria-label="Comparison range">
        {#each RANGES as [value, label]}
          <button class="pill" class:on={days === value} aria-pressed={days === value} onclick={() => (days = value)}>{label}</button>
        {/each}
      </div>
    </div>
  </header>

  {#if notice}<div class="notice" role="status">{notice}</div>{/if}
  {#if error}
    <div class="errorband" role="alert">
      <p class="error">{error}</p>
      <button onclick={retry}>Retry</button>
    </div>
  {/if}

  {#if catalogReady && catalog.length >= 2}
    <section class="pickerband" aria-label="Choose models">
      <div class="sidepick" style="--side:{colors[0]}">
        <label for="compare-a"><span>A</span> First model</label>
        <select id="compare-a" value={urlA} onchange={(e) => changeSide('a', e)}>
          {#each catalog as model}
            <option value={model.model} disabled={model.model === urlB}>{model.model} · {fmtTokens(model.tokens)}</option>
          {/each}
        </select>
      </div>
      <button class="swap" onclick={swap} disabled={!urlA || !urlB} aria-label="Swap models" title="Swap models">⇄</button>
      <div class="sidepick" style="--side:{colors[1]}">
        <label for="compare-b"><span>B</span> Second model</label>
        <select id="compare-b" value={urlB} onchange={(e) => changeSide('b', e)}>
          {#each catalog as model}
            <option value={model.model} disabled={model.model === urlA}>{model.model} · {fmtTokens(model.tokens)}</option>
          {/each}
        </select>
      </div>
    </section>
  {/if}

  {#if !catalogReady || (loading && !result)}
    <div class="skeleton" aria-label="Loading comparison">
      <div></div><div></div><div></div><div></div>
      <span>building the matchup…</span>
    </div>
  {:else if catalog.length < 2}
    <div class="empty">
      <strong>Two models make a matchup.</strong>
      <p>Use at least two visible models in your harnesses, then come back here.</p>
      <a href="/models">Open Models</a>
    </div>
  {:else if result}
    {@const a = result.a}
    {@const b = result.b}
    {@const sharedProjectSet = sharedProjects([a, b])}
    {@const sharedSourceSet = sharedSources([a, b])}
    {@const latestChange = result.rivalry.lead_changes[result.rivalry.lead_changes.length - 1]}

    <section class="mileage up">
      <div class="sectionhead tight">
        <h2>{hero?.title ?? 'Cost mileage'} <span class="head-range">{RANGE_LABEL[days]}</span></h2>
        <div class="pills" aria-label="Anchor token amount">
          {#each ANCHORS as value}
            <button class="pill" class:on={anchorTokens === value} aria-pressed={anchorTokens === value} onclick={() => (anchorTokens = value)}>{fmtTokens(value)}</button>
          {/each}
        </div>
      </div>

      {#if hero}
        <div class="mileage-grid">
          <div class="mile" style="--side:{colors[0]}">
            <a class="mile-name" href={modelUrl(a.model)}><span>A</span>{a.model}</a>
            <div class="mile-number"><AnimatedNumber value={hero.aTokens} format={fmtTokens} /></div>
            <div class="mile-unit">tokens</div>
            <div class="mile-cost"><AnimatedNumber value={hero.aCost} format={fmtCost} /> <span>est.</span></div>
            <div class="mile-track"><div class="gw" style="width:{(hero.aTokens / Math.max(hero.aTokens, hero.bTokens)) * 100}%;background:{colors[0]}"></div></div>
            <div class="mile-rate">{fmtRate(a.cost_per_mtok)}</div>
          </div>
          <div class="relation">
            <strong>{hero.paid ? '≈' : 'VS'}</strong>
            <span>{hero.paid ? 'same est. cost' : 'same token amount'}</span>
          </div>
          <div class="mile" style="--side:{colors[1]}">
            <a class="mile-name" href={modelUrl(b.model)}><span>B</span>{b.model}</a>
            <div class="mile-number"><AnimatedNumber value={hero.bTokens} format={fmtTokens} /></div>
            <div class="mile-unit">tokens</div>
            <div class="mile-cost"><AnimatedNumber value={hero.bCost} format={fmtCost} /> <span>est.</span></div>
            <div class="mile-track"><div class="gw" style="width:{(hero.bTokens / Math.max(hero.aTokens, hero.bTokens)) * 100}%;background:{colors[1]}"></div></div>
            <div class="mile-rate">{fmtRate(b.cost_per_mtok)}</div>
          </div>
        </div>
      {:else}
        <div class="unavailable">
          <strong>Not enough priced usage for cost mileage.</strong>
          <span>A {fmtRate(a.cost_per_mtok)} · B {fmtRate(b.cost_per_mtok)}</span>
          <p>Unpriced and app-estimated rows never acquire a cost.</p>
        </div>
      {/if}
    </section>

    <section class="scorecard up">
      <div class="sectionhead tight">
        <h2>Tale of the tape</h2>
        <span class="fig">FIG. 01</span>
      </div>
      <div class="tablewrap">
        <table>
          <thead><tr><th>Metric</th><th class="num"><span class="thead-a">A</span> {a.model}</th><th class="num"><span class="thead-b">B</span> {b.model}</th><th class="num">Difference</th></tr></thead>
          <tbody>
            <tr><td>Recorded tokens</td><td class="num">{fmtTokensExact(a.tokens)}</td><td class="num">{fmtTokensExact(b.tokens)}</td><td class="num delta">{delta(a.tokens, b.tokens)}</td></tr>
            <tr><td>Est. cost on priced rows</td><td class="num">{fmtCostExact(a.priced_cost_usd)}</td><td class="num">{fmtCostExact(b.priced_cost_usd)}</td><td class="num delta">Coverage {fmtPct(coverage(a))} / {fmtPct(coverage(b))}</td></tr>
            <tr><td>Calls</td><td class="num">{a.events.toLocaleString()}</td><td class="num">{b.events.toLocaleString()}</td><td class="num delta">{delta(a.events, b.events)}</td></tr>
            <tr><td>Sessions</td><td class="num">{a.sessions.toLocaleString()}</td><td class="num">{b.sessions.toLocaleString()}</td><td class="num delta">{delta(a.sessions, b.sessions)}</td></tr>
            <tr><td>Share of all visible usage</td><td class="num">{fmtPct(percent(a.tokens, result.total_window_tokens))}</td><td class="num">{fmtPct(percent(b.tokens, result.total_window_tokens))}</td><td class="num delta">{delta(percent(a.tokens, result.total_window_tokens), percent(b.tokens, result.total_window_tokens))}</td></tr>
            <tr><td>First recorded in window</td><td class="num">{fmtDate(a.first_ts)}</td><td class="num">{fmtDate(b.first_ts)}</td><td class="num delta">—</td></tr>
            <tr><td>Last used</td><td class="num">{fmtDate(a.last_ts)}</td><td class="num">{fmtDate(b.last_ts)}</td><td class="num delta">—</td></tr>
          </tbody>
        </table>
      </div>
      {#if a.estimated_tokens + b.estimated_tokens > 0}
        <div class="caveat"><span class="tag est">app-estimated</span> Recorded usage includes {fmtTokens(a.estimated_tokens)} for A and {fmtTokens(b.estimated_tokens)} for B. These tokens remain separate from measured cost and speed.</div>
      {/if}
    </section>

    <section class="race up">
      <div class="sectionhead">
        <div><h2>Token race{raceRange ? ` — ${raceRange}` : ''}</h2><p>Two series, one calendar, zero-based scale</p></div>
        <div class="pills" aria-label="Race chart mode">
          <button class="pill" class:on={raceMode === 'daily'} aria-pressed={raceMode === 'daily'} onclick={() => (raceMode = 'daily')}>Daily</button>
          <button class="pill" class:on={raceMode === 'cumulative'} aria-pressed={raceMode === 'cumulative'} onclick={() => (raceMode = 'cumulative')}>Cumulative</button>
        </div>
      </div>
      <div class="chart-legend">
        <span><i style="background:{colors[0]}"></i>A · {a.model}</span>
        <span><i class="dashed" style="border-color:{colors[1]}"></i>B · {b.model}</span>
      </div>
      {#if raceOption}
        <Chart option={raceOption} height={290} />
      {:else}
        <div class="loading">no recorded usage for either model in this range</div>
      {/if}
      <div class="rivalry-grid">
        <div class="rivalstat">
          <span>Daily wins</span>
          <strong><AnimatedNumber value={result.rivalry.a_wins} /> <i>—</i> <AnimatedNumber value={result.rivalry.b_wins} /></strong>
          <small>A — B · {result.rivalry.ties} positive {result.rivalry.ties === 1 ? 'tie' : 'ties'}</small>
        </div>
        <div class="rivalstat">
          <span>Cumulative lead changes</span>
          <strong><AnimatedNumber value={result.rivalry.lead_changes.length} /></strong>
          <small>{latestChange ? `${latestChange.direction === 'a' ? a.model : b.model} took the lead ${fmtDay(latestChange.date)}` : 'no overtake after the first leader'}</small>
        </div>
        <div class="rivalstat">
          <span>Biggest daily gap</span>
          <strong>{result.rivalry.biggest_gap ? fmtTokens(result.rivalry.biggest_gap.margin) : '—'}</strong>
          <small>{result.rivalry.biggest_gap ? `${result.rivalry.biggest_gap.wider_side === 'a' ? a.model : b.model} · ${fmtDay(result.rivalry.biggest_gap.date)}` : 'no day separated them'}</small>
        </div>
      </div>
    </section>

    <div class="split2">
      <section class="mixpanel up">
        <div class="sectionhead tight"><h2>Recorded token mix</h2><span class="fig">FIG. 02</span></div>
        <div class="mixlegend">
          {#each ['Fresh input', 'Output', 'Cache read', 'Cache write'] as label, i}
            <span><i style="background:{MIX_COLORS[i]}"></i>{label}</span>
          {/each}
        </div>
        {#each [a, b] as model, index}
          <div class="mixrow">
            <div class="mixname"><span>{index === 0 ? 'A' : 'B'}</span>{model.model}<b>{fmtTokens(model.tokens)}</b></div>
            {#if model.tokens > 0}
              <div class="mixbar" aria-label={`${model.model} recorded token mix`}>
                {#each mix(model) as part}
                  {#if part.tokens > 0}<i style="width:{part.share}%;background:{part.color}" title={`${part.name}: ${fmtTokensExact(part.tokens)} (${part.share.toFixed(1)}%)`}></i>{/if}
                {/each}
              </div>
              <div class="mixvalues">
                {#each mix(model) as part}<span>{part.share > 0 && part.share < 0.05 ? '<0.1' : part.share.toFixed(1)}%</span>{/each}
              </div>
            {:else}
              <div class="mixempty">no recorded usage in this range</div>
            {/if}
          </div>
        {/each}
      </section>

      <section class="efficiency up">
        <div class="sectionhead tight"><h2>Efficiency duels</h2><span class="fig">FIG. 03</span></div>
        <div class="tablewrap">
          <table>
            <thead><tr><th>Metric</th><th class="num">A</th><th class="num">B</th><th class="num">Delta</th></tr></thead>
            <tbody>
              <tr><td>Recorded tokens / call</td><td class="num">{a.events ? fmtTokens(a.tokens / a.events) : '—'}</td><td class="num">{b.events ? fmtTokens(b.tokens / b.events) : '—'}</td><td class="num delta">{delta(a.events ? a.tokens / a.events : null, b.events ? b.tokens / b.events : null)}</td></tr>
              <tr><td>Generated output / call</td><td class="num">{a.events ? fmtTokens(a.generated_output / a.events) : '—'}</td><td class="num">{b.events ? fmtTokens(b.generated_output / b.events) : '—'}</td><td class="num delta">{delta(a.events ? a.generated_output / a.events : null, b.events ? b.generated_output / b.events : null)}</td></tr>
              <tr><td>Context tokens / call</td><td class="num">{a.events ? fmtTokens((a.input_tokens + a.cache_read_tokens + a.cache_write_tokens) / a.events) : '—'}</td><td class="num">{b.events ? fmtTokens((b.input_tokens + b.cache_read_tokens + b.cache_write_tokens) / b.events) : '—'}</td><td class="num delta">{delta(a.events ? (a.input_tokens + a.cache_read_tokens + a.cache_write_tokens) / a.events : null, b.events ? (b.input_tokens + b.cache_read_tokens + b.cache_write_tokens) / b.events : null)}</td></tr>
              <tr><td>Cache-served input</td><td class="num">{fmtPct(percent(a.cache_read_tokens, a.input_tokens + a.cache_read_tokens))}</td><td class="num">{fmtPct(percent(b.cache_read_tokens, b.input_tokens + b.cache_read_tokens))}</td><td class="num delta">{delta(percent(a.cache_read_tokens, a.input_tokens + a.cache_read_tokens), percent(b.cache_read_tokens, b.input_tokens + b.cache_read_tokens))}</td></tr>
              <tr><td>Subagent token share</td><td class="num">{fmtPct(percent(a.subagent_tokens, a.tokens))}</td><td class="num">{fmtPct(percent(b.subagent_tokens, b.tokens))}</td><td class="num delta">{delta(percent(a.subagent_tokens, a.tokens), percent(b.subagent_tokens, b.tokens))}</td></tr>
              <tr><td>Est. cost / priced call</td><td class="num">{fmtCost(costPerCall(a))}</td><td class="num">{fmtCost(costPerCall(b))}</td><td class="num delta">{delta(costPerCall(a), costPerCall(b), true)}</td></tr>
            </tbody>
          </table>
        </div>
      </section>
    </div>

    <section class="rhythm up">
      <div class="sectionhead tight"><h2>Rhythm and habits</h2></div>
      <div class="chart-legend">
        <span><i style="background:{colors[0]}"></i>A · {a.model}</span>
        <span><i class="dashed" style="border-color:{colors[1]}"></i>B · {b.model}</span>
      </div>
      {#if hourOption}<Chart option={hourOption} height={240} />{:else}<div class="loading">no activity in this range</div>{/if}
      <div class="tablewrap habit-table">
        <table>
          <thead><tr><th>Habit</th><th class="num">A · {a.model}</th><th class="num">B · {b.model}</th><th class="num">Difference</th></tr></thead>
          <tbody>
            <tr><td>Active days</td><td class="num">{a.active_days}</td><td class="num">{b.active_days}</td><td class="num delta">{delta(a.active_days, b.active_days)}</td></tr>
            <tr><td>Tokens / active day</td><td class="num">{a.active_days ? fmtTokens(a.tokens / a.active_days) : '—'}</td><td class="num">{b.active_days ? fmtTokens(b.tokens / b.active_days) : '—'}</td><td class="num delta">{delta(a.active_days ? a.tokens / a.active_days : null, b.active_days ? b.tokens / b.active_days : null)}</td></tr>
            <tr><td>Current streak</td><td class="num">{a.current_streak}d</td><td class="num">{b.current_streak}d</td><td class="num delta">{delta(a.current_streak, b.current_streak)}</td></tr>
            <tr><td>Longest streak</td><td class="num">{a.longest_streak}d</td><td class="num">{b.longest_streak}d</td><td class="num delta">{delta(a.longest_streak, b.longest_streak)}</td></tr>
            <tr><td>Peak day</td><td class="num">{fmtDay(a.peak_day)}<small>{fmtTokens(a.peak_day_tokens)}</small></td><td class="num">{fmtDay(b.peak_day)}<small>{fmtTokens(b.peak_day_tokens)}</small></td><td class="num delta">{delta(a.peak_day_tokens, b.peak_day_tokens)}</td></tr>
            <tr><td>Night-owl share</td><td class="num">{fmtPct(a.tokens ? a.night_share * 100 : null)}</td><td class="num">{fmtPct(b.tokens ? b.night_share * 100 : null)}</td><td class="num delta">{delta(a.tokens ? a.night_share * 100 : null, b.tokens ? b.night_share * 100 : null)}</td></tr>
            <tr><td>Busiest hour</td><td class="num">{fmtHour(a.busiest_hour)}</td><td class="num">{fmtHour(b.busiest_hour)}</td><td class="num delta">—</td></tr>
          </tbody>
        </table>
      </div>
    </section>

    {#if a.timing.timed_calls + b.timing.timed_calls > 0}
      <section class="speed up">
        <div class="sectionhead"><div><h2>Reported speed</h2><p>Harness telemetry, not a provider benchmark</p></div><span class="fig">20+ CALLS EACH FOR DELTA</span></div>
        <div class="timing-sources">
          {#each [a, b] as model, index}
            <div><strong>{index === 0 ? 'A' : 'B'} timing sources</strong>{#if model.timing.sources.length}{#each model.timing.sources as source}<span class="stag"><i style="background:{sourceSwatch(source.source)}"></i>{sourceLabel(source.source)} · {source.timed_calls}</span>{/each}{:else}<span class="muted">not reported</span>{/if}</div>
          {/each}
        </div>
        <div class="tablewrap">
          <table>
            <thead><tr><th>Measure</th><th class="num">A</th><th class="num">Coverage</th><th class="num">B</th><th class="num">Coverage</th><th class="num">Delta</th></tr></thead>
            <tbody>
              <tr><td>Avg response duration</td><td class="num">{a.timing.avg_duration_ms === null ? 'not reported' : fmtDuration(Math.round(a.timing.avg_duration_ms))}</td><td class="num">{a.timing.timed_calls} / {a.events}</td><td class="num">{b.timing.avg_duration_ms === null ? 'not reported' : fmtDuration(Math.round(b.timing.avg_duration_ms))}</td><td class="num">{b.timing.timed_calls} / {b.events}</td><td class="num delta">{speedDelta(a.timing.avg_duration_ms, b.timing.avg_duration_ms, a.timing.timed_calls, b.timing.timed_calls, true)}</td></tr>
              <tr><td>Avg time to first token</td><td class="num">{a.timing.avg_ttft_ms === null ? 'not reported' : fmtDuration(Math.round(a.timing.avg_ttft_ms))}</td><td class="num">{a.timing.ttft_calls} / {a.events}</td><td class="num">{b.timing.avg_ttft_ms === null ? 'not reported' : fmtDuration(Math.round(b.timing.avg_ttft_ms))}</td><td class="num">{b.timing.ttft_calls} / {b.events}</td><td class="num delta">{speedDelta(a.timing.avg_ttft_ms, b.timing.avg_ttft_ms, a.timing.ttft_calls, b.timing.ttft_calls, true)}</td></tr>
              <tr><td>End-to-end output / sec</td><td class="num">{a.timing.end_to_end_output_tps === null ? 'not reported' : `${a.timing.end_to_end_output_tps.toFixed(1)} tok/s`}</td><td class="num">{a.timing.timed_calls} / {a.events}</td><td class="num">{b.timing.end_to_end_output_tps === null ? 'not reported' : `${b.timing.end_to_end_output_tps.toFixed(1)} tok/s`}</td><td class="num">{b.timing.timed_calls} / {b.events}</td><td class="num delta">{speedDelta(a.timing.end_to_end_output_tps, b.timing.end_to_end_output_tps, a.timing.timed_calls, b.timing.timed_calls)}</td></tr>
              <tr><td>Generation-only output / sec</td><td class="num">{a.timing.generation_output_tps === null ? 'not reported' : `${a.timing.generation_output_tps.toFixed(1)} tok/s`}</td><td class="num">{a.timing.generation_calls} / {a.events}</td><td class="num">{b.timing.generation_output_tps === null ? 'not reported' : `${b.timing.generation_output_tps.toFixed(1)} tok/s`}</td><td class="num">{b.timing.generation_calls} / {b.events}</td><td class="num delta">{speedDelta(a.timing.generation_output_tps, b.timing.generation_output_tps, a.timing.generation_calls, b.timing.generation_calls)}</td></tr>
            </tbody>
          </table>
        </div>
        <div class="caveat">Positive, non-estimated, non-failed/non-cancelled calls only. Output rates use ratios of sums. Antigravity duration may describe a whole step; source labels are part of the result.</div>
      </section>
    {/if}

    <section class="home up">
      <div class="sectionhead tight"><h2>Home turf</h2><span class="fig">TOP 5 EACH</span></div>
      <div class="homegrid">
        {#each [a, b] as model, index}
          <div class="turf">
            <div class="turf-title"><span style="background:{colors[index]}">{index === 0 ? 'A' : 'B'}</span><a href={modelUrl(model.model)}>{model.model}</a></div>
            <h3>Harnesses · {model.by_source.length}</h3>
            {#each model.by_source.slice(0, 5) as source}
              <div class="turfrow"><span class="stag"><i style="background:{sourceSwatch(source.source)}"></i>{sourceLabel(source.source)}</span>{#if sharedSourceSet.has(source.source)}<span class="sharedmark">shared</span>{/if}<b>{fmtTokens(source.tokens)}</b><small>{fmtPct(percent(source.tokens, model.tokens))}</small></div>
            {:else}<p class="muted">No harness usage in this range.</p>{/each}
            <h3>Projects · {model.by_project.filter((p) => p.project !== 'unknown').length}</h3>
            {#each model.by_project.slice(0, 5) as project}
              <div class="turfrow">
                {#if project.project === 'unknown'}<span class="projectname">Unknown</span>{:else}<a class="projectname" href={projectUrl(project.project)} title={project.project}>{basename(project.project)}</a>{/if}
                {#if sharedProjectSet.has(project.project)}<span class="sharedmark">shared</span>{/if}<b>{fmtTokens(project.tokens)}</b><small>{projectShare(project, model)}</small>
              </div>
            {:else}<p class="muted">No project usage in this range.</p>{/each}
          </div>
        {/each}
      </div>
      <div class="noteband"><span class="fg">MATCHUP</span><p>{sharedProjectSet.size ? `They meet in ${sharedProjectSet.size} ${sharedProjectSet.size === 1 ? 'project' : 'projects'}.` : 'They never share a named project in this range.'} <b>{a.model}</b> favours {a.by_project[0] ? basename(a.by_project[0].project) : 'no project'}; <b>{b.model}</b> favours {b.by_project[0] ? basename(b.by_project[0].project) : 'no project'}.</p></div>
    </section>
  {/if}
</div>

<style>
  .cframe { flex: 1; min-width: 0; }
  .cframe :is(button, a, select):focus-visible { outline: 3px solid var(--vio); outline-offset: 2px; }
  .thd { display: flex; align-items: center; justify-content: space-between; gap: 16px; padding: 16px clamp(22px, 1.8vw, 40px) 14px; border-bottom: 2px solid var(--ink); flex-wrap: wrap; }
  .thd h1 { margin: 0; font-size: clamp(30px, 2.6vw, 46px); }
  .head-controls { display: flex; align-items: center; gap: 12px; }
  .updating { font: 500 10px/1 var(--font-mono); color: var(--dim); }
  .notice, .errorband { padding: 12px clamp(22px, 1.8vw, 40px); border-bottom: 2px solid var(--ink); }
  .notice { background: var(--acd); font: 500 12px/1.4 var(--font-ui); }
  .errorband { display: flex; align-items: center; justify-content: space-between; gap: 16px; }
  .errorband p { margin: 0; }

  .pickerband { display: grid; grid-template-columns: minmax(0, 1fr) 64px minmax(0, 1fr); align-items: stretch; border-bottom: 2px solid var(--ink); }
  .sidepick { min-width: 0; padding: 14px clamp(22px, 1.8vw, 40px) 16px; border-top: 6px solid var(--side); }
  .sidepick label { display: flex; align-items: center; gap: 8px; margin-bottom: 8px; font: 600 10px/1 var(--font-ui); letter-spacing: 1.1px; text-transform: uppercase; }
  .sidepick label span { display: inline-flex; align-items: center; justify-content: center; width: 24px; height: 24px; background: var(--side); color: var(--ink); font: 400 16px/1 var(--font-disp); }
  .sidepick select { width: 100%; min-width: 0; height: 40px; padding: 0 10px; border: 2px solid var(--ink); border-radius: 0; background: var(--bone); color: var(--ink); font: 500 12px/1 var(--font-mono); cursor: pointer; }
  .swap { align-self: center; justify-self: center; width: 44px; height: 40px; padding: 0; font-size: 20px; }

  .mileage, .scorecard, .race, .rhythm, .speed, .home { border-bottom: 2px solid var(--ink); }
  .sectionhead { display: flex; align-items: center; justify-content: space-between; gap: 16px; padding: 15px clamp(22px, 1.8vw, 40px) 12px; flex-wrap: wrap; }
  .sectionhead h2 { margin: 0; font: 600 12px/1.2 var(--font-ui); letter-spacing: 1.5px; text-transform: uppercase; }
  .sectionhead p { margin: 5px 0 0; color: var(--dim); font: 400 10px/1.4 var(--font-mono); }
  .sectionhead.tight { padding-top: 12px; padding-bottom: 10px; }
  .head-range { margin-left: 8px; font: 400 10px/1 var(--font-mono); letter-spacing: 0.8px; opacity: 0.55; }
  .mileage-grid { display: grid; grid-template-columns: minmax(0, 1fr) 110px minmax(0, 1fr); border-top: 2px solid var(--ink); }
  .mile { min-width: 0; padding: clamp(18px, 2vw, 34px) clamp(22px, 2.4vw, 48px); border-top: 7px solid var(--side); }
  .mile-name { display: flex; align-items: center; gap: 9px; min-width: 0; font: 600 clamp(12px, 1vw, 16px)/1.3 var(--font-ui); overflow-wrap: break-word; }
  .mile-name:hover { color: var(--org); }
  .mile-name span { display: inline-flex; align-items: center; justify-content: center; flex: none; width: 26px; height: 26px; background: var(--side); font: 400 17px/1 var(--font-disp); }
  .mile-number { margin-top: 16px; font: 400 clamp(64px, 6vw, 112px)/0.9 var(--font-disp); letter-spacing: -2px; font-variant-numeric: tabular-nums; }
  .mile-unit { margin-top: 7px; color: var(--dim); font: 500 11px/1 var(--font-mono); text-transform: uppercase; letter-spacing: 1.3px; }
  .mile-cost { margin-top: 18px; font: 400 clamp(24px, 2vw, 38px)/1 var(--font-disp); }
  .mile-cost span { color: var(--dim); font: 500 10px/1 var(--font-mono); text-transform: uppercase; }
  .mile-track { height: 16px; margin-top: 16px; background: var(--hair); }
  .mile-track > div { height: 100%; }
  .mile-rate { margin-top: 8px; color: var(--dim); font: 500 10px/1.4 var(--font-mono); }
  .relation { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 12px; border-left: 2px solid var(--ink); border-right: 2px solid var(--ink); background: var(--ink); color: var(--bone); text-align: center; }
  .relation strong { font: 400 64px/1 var(--font-disp); color: var(--org); }
  .relation span { padding: 0 8px; font: 500 9px/1.5 var(--font-mono); text-transform: uppercase; letter-spacing: 1px; }
  .caveat { padding: 11px clamp(22px, 1.8vw, 40px); color: var(--dim); font: 400 10px/1.55 var(--font-mono); }
  .unavailable { padding: 34px clamp(22px, 1.8vw, 40px); border-top: 2px solid var(--ink); }
  .unavailable strong { display: block; font: 400 clamp(24px, 2.2vw, 42px)/1.1 var(--font-disp); text-transform: uppercase; }
  .unavailable span { display: block; margin-top: 12px; font: 500 12px/1.4 var(--font-mono); }
  .unavailable p { margin: 8px 0 0; color: var(--dim); }

  .tablewrap { width: 100%; overflow-x: auto; }
  .tablewrap table { min-width: 610px; }
  .tablewrap th, .tablewrap td { padding-left: clamp(14px, 1.8vw, 40px); padding-right: clamp(14px, 1.8vw, 40px); }
  .tablewrap td small { display: block; margin-top: 3px; color: var(--dim); font-size: 9px; }
  .thead-a, .thead-b { display: inline-flex; align-items: center; justify-content: center; width: 18px; height: 18px; color: var(--bone); background: var(--ink); }
  .delta { color: var(--org); white-space: nowrap; }
  .caveat { border-top: 1px solid var(--hair); }
  .caveat .tag { margin-right: 6px; }

  .chart-legend, .mixlegend { display: flex; gap: 16px; flex-wrap: wrap; padding: 0 clamp(22px, 1.8vw, 40px) 8px; }
  .chart-legend span, .mixlegend span { display: inline-flex; align-items: center; gap: 6px; min-width: 0; color: var(--dim); font: 500 10px/1.3 var(--font-mono); }
  .chart-legend i, .mixlegend i { display: inline-block; flex: none; width: 10px; height: 10px; }
  .chart-legend i.dashed { width: 18px; height: 0; background: none; border-top: 2px dashed; }
  .rivalry-grid { display: grid; grid-template-columns: repeat(3, 1fr); border-top: 2px solid var(--ink); }
  .rivalstat { min-width: 0; padding: 16px clamp(22px, 1.8vw, 40px); border-right: 2px solid var(--ink); }
  .rivalstat:last-child { border-right: none; }
  .rivalstat > span { font: 600 9px/1 var(--font-ui); text-transform: uppercase; letter-spacing: 1.2px; opacity: 0.55; }
  .rivalstat strong { display: block; margin-top: 9px; font: 400 clamp(30px, 2.6vw, 48px)/1 var(--font-disp); }
  .rivalstat strong i { color: var(--dim); font-style: normal; }
  .rivalstat small { display: block; margin-top: 7px; color: var(--dim); font: 400 10px/1.45 var(--font-mono); overflow-wrap: break-word; }

  .split2 { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); border-bottom: 2px solid var(--ink); }
  .split2 > section { min-width: 0; }
  .split2 > section:first-child { border-right: 2px solid var(--ink); }
  .mixrow { padding: 0 clamp(22px, 1.8vw, 40px) 18px; }
  .mixname { display: flex; align-items: center; gap: 8px; min-width: 0; margin-bottom: 7px; font: 600 12px/1.3 var(--font-ui); }
  .mixname > span { display: inline-flex; align-items: center; justify-content: center; width: 20px; height: 20px; background: var(--ink); color: var(--bone); font: 400 13px/1 var(--font-disp); }
  .mixname b { margin-left: auto; font: 500 11px/1 var(--font-mono); }
  .mixbar { display: flex; width: 100%; height: 26px; border: 2px solid var(--ink); background: var(--hair); }
  .mixbar i { display: block; height: 100%; }
  .mixvalues { display: grid; grid-template-columns: repeat(4, 1fr); margin-top: 5px; }
  .mixvalues span { text-align: center; color: var(--dim); font: 500 9px/1 var(--font-mono); }
  .mixempty { padding: 14px 0; color: var(--dim); font: 500 10px/1 var(--font-mono); letter-spacing: 0.8px; text-transform: uppercase; border: 2px solid var(--hair); }
  .efficiency .tablewrap table { min-width: 520px; }
  .efficiency .tablewrap th, .efficiency .tablewrap td { padding-left: 12px; padding-right: 12px; }

  .habit-table { border-top: 2px solid var(--ink); }
  .timing-sources { display: grid; grid-template-columns: 1fr 1fr; border-top: 2px solid var(--ink); border-bottom: 1px solid var(--hair); }
  .timing-sources > div { display: flex; align-items: center; gap: 7px; flex-wrap: wrap; padding: 12px clamp(22px, 1.8vw, 40px); }
  .timing-sources > div:first-child { border-right: 2px solid var(--ink); }
  .timing-sources strong { margin-right: 4px; font: 600 10px/1 var(--font-ui); text-transform: uppercase; letter-spacing: 1px; }

  .homegrid { display: grid; grid-template-columns: 1fr 1fr; border-top: 2px solid var(--ink); }
  .turf { min-width: 0; padding: 18px clamp(22px, 1.8vw, 40px) 20px; }
  .turf:first-child { border-right: 2px solid var(--ink); }
  .turf-title { display: flex; align-items: center; gap: 9px; min-width: 0; margin-bottom: 18px; font: 600 14px/1.3 var(--font-ui); }
  .turf-title > span { display: inline-flex; align-items: center; justify-content: center; flex: none; width: 26px; height: 26px; font: 400 17px/1 var(--font-disp); }
  .turf-title a { overflow-wrap: break-word; }
  .turf h3 { margin: 16px 0 8px; font: 600 10px/1 var(--font-ui); text-transform: uppercase; letter-spacing: 1.2px; }
  .turfrow { display: flex; align-items: center; gap: 7px; min-width: 0; padding: 8px 0; border-bottom: 1px solid var(--hair); }
  .turfrow:last-child { border-bottom: none; }
  .turfrow .stag, .projectname { min-width: 0; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .projectname { font: 500 12px/1.3 var(--font-ui); }
  a.projectname:hover { color: var(--org); }
  .turfrow b { font: 500 11px/1 var(--font-mono); }
  .turfrow small { width: 44px; text-align: right; color: var(--dim); font: 400 9px/1 var(--font-mono); }
  .sharedmark { padding: 2px 4px; border: 1px solid var(--ink); background: var(--acd); font: 500 8px/1 var(--font-mono); text-transform: uppercase; }
  .home .noteband { border-left: none; border-right: none; border-bottom: none; }

  .skeleton { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; padding: 30px clamp(22px, 1.8vw, 40px); }
  .skeleton div { height: 150px; border: 2px solid var(--hair); background: var(--hair); }
  .skeleton span { grid-column: 1 / -1; text-align: center; color: var(--dim); font: 500 11px/1 var(--font-mono); }
  .empty { margin: 34px clamp(22px, 1.8vw, 40px); padding: 34px; border: 2px solid var(--ink); }
  .empty strong { display: block; font: 400 clamp(28px, 3vw, 52px)/1 var(--font-disp); text-transform: uppercase; }
  .empty p { color: var(--dim); }
  .empty a { display: inline-block; padding: 9px 13px; border: 2px solid var(--ink); background: var(--org); color: #fff; font: 600 11px/1 var(--font-ui); text-transform: uppercase; }

  @media (max-width: 1100px) {
    .split2 { grid-template-columns: 1fr; }
    .split2 > section:first-child { border-right: none; border-bottom: 2px solid var(--ink); }
    .mile-number { font-size: clamp(54px, 7vw, 76px); }
    .mileage-grid { grid-template-columns: minmax(0, 1fr) 84px minmax(0, 1fr); }
    .relation strong { font-size: 48px; }
  }

  @media (max-width: 900px) {
    .pickerband { grid-template-columns: 1fr; }
    .sidepick { padding: 12px 20px; }
    .swap { margin: 8px auto; }
    .mileage-grid { grid-template-columns: 1fr; }
    .relation { min-height: 76px; border: none; border-top: 2px solid var(--ink); border-bottom: 2px solid var(--ink); }
    .rivalry-grid, .timing-sources, .homegrid { grid-template-columns: 1fr; }
    .rivalstat, .timing-sources > div:first-child, .turf:first-child { border-right: none; border-bottom: 2px solid var(--ink); }
    .rivalstat:last-child { border-bottom: none; }
    .skeleton { grid-template-columns: 1fr; }
    .skeleton span { grid-column: 1; }
  }
</style>
