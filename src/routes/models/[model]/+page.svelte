<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/stores';
  import type { EChartsOption } from 'echarts';
  import Chart from '$lib/Chart.svelte';
  import AnimatedNumber from '$lib/AnimatedNumber.svelte';
  import { api, type ModelDetail, type Overview } from '$lib/api';
  import {
    fmtCost,
    fmtDate,
    fmtDateTime,
    fmtTokens,
    MODEL_PALETTE,
    MIX_COLORS,
    sourceColor,
    sourceLabel,
    basename,
  } from '$lib/format';
  import { TOOLTIP, ANIM, AXIS_LABEL, AXIS_LINE, SPLIT_LINE, donutSeries, dateTick } from '$lib/chartTheme';

  let detail = $state<ModelDetail | null>(null);
  let overview = $state<Overview | null>(null);
  let error = $state('');
  let loading = $state(true);

  const name = $derived($page.params.model ?? '');

  async function load() {
    if (!name) return;
    loading = true;
    try {
      const [d, o] = await Promise.all([api.modelDetail(name), api.overview()]);
      detail = d;
      overview = o;
      error = '';
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    name;
    load();
  });

  onMount(() => {
    const h = () => load();
    window.addEventListener('tt-sync', h);
    return () => window.removeEventListener('tt-sync', h);
  });

  const usagePct = $derived.by(() => {
    if (!detail || !overview || !overview.total_tokens) return null;
    return ((detail.tokens / overview.total_tokens) * 100).toFixed(1);
  });

  const donutOption = $derived.by(() => {
    if (!detail || !detail.tokens) return undefined;
    const data = [
      { name: 'Input', value: detail.input_tokens, itemStyle: { color: MIX_COLORS[0] } },
      { name: 'Output', value: detail.output_tokens, itemStyle: { color: MIX_COLORS[1] } },
      { name: 'Cache read', value: detail.cache_read_tokens, itemStyle: { color: MIX_COLORS[2] } },
      { name: 'Cache write', value: detail.cache_write_tokens, itemStyle: { color: MIX_COLORS[3] } },
    ].filter((d) => d.value > 0);
    if (!data.length) return undefined;
    return {
      backgroundColor: 'transparent',
      ...ANIM,
      tooltip: {
        ...TOOLTIP,
        formatter: (p: any) => `${p.name}<br/>${fmtTokens(p.value)} (${p.percent}%)`,
      },
      series: [donutSeries(data)],
    } satisfies EChartsOption;
  });

  const dailyOption = $derived.by(() => {
    if (!detail || !detail.daily.length) return undefined;
    return {
      backgroundColor: 'transparent',
      ...ANIM,
      animationDelay: (idx: number) => idx * 8,
      tooltip: {
        trigger: 'axis',
        ...TOOLTIP,
        valueFormatter: (v: unknown) => fmtTokens(Number(v)),
      },
      grid: { left: 8, right: 8, top: 20, bottom: 0, containLabel: true },
      xAxis: {
        type: 'category',
        data: detail.daily.map((d) => d.date),
        axisLabel: { ...AXIS_LABEL, hideOverlap: true, formatter: dateTick },
        axisLine: AXIS_LINE,
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
          data: detail.daily.map((d) => d.tokens),
          itemStyle: { color: MODEL_PALETTE[0] },
          animationDelay: (idx: number) => idx * 8,
        },
      ],
    } satisfies EChartsOption;
  });
</script>

<a class="backlink morelink" href="/models">← All models</a>

{#if error}
  <p class="error">{error}</p>
{:else if loading}
  <div class="loading">loading model card…</div>
{:else if !detail}
  <div class="loading">No usage recorded for this model</div>
{:else}
  <h1 class="up">{detail.model}</h1>
  <p class="sub">
    {detail.events.toLocaleString()} calls across
    {detail.by_source.length} harness{detail.by_source.length !== 1 ? 'es' : ''}
    {#if detail.first_ts}
      since {fmtDate(detail.first_ts)}
    {/if}
  </p>

  <div class="cards">
    <div class="card up">
      <div class="label">Total tokens</div>
      <div class="value"><AnimatedNumber value={detail.tokens} format={fmtTokens} /></div>
      <div class="hint">
        {fmtTokens(detail.input_tokens)} in · {fmtTokens(detail.output_tokens)} out
      </div>
    </div>
    <div class="card up" style="animation-delay:60ms">
      <div class="label">Est. cost</div>
      <div class="value"><AnimatedNumber value={detail.cost_usd ?? 0} format={fmtCost} /></div>
      <div class="hint">API-equivalent estimate at list prices</div>
    </div>
    <div class="card up" style="animation-delay:120ms">
      <div class="label">Calls</div>
      <div class="value"><AnimatedNumber value={detail.events} /></div>
      <div class="hint">{detail.sessions.toLocaleString()} session{detail.sessions !== 1 ? 's' : ''}</div>
    </div>
    <div class="card up" style="animation-delay:180ms">
      <div class="label">Avg tokens / call</div>
      <div class="value">
        <AnimatedNumber value={detail.events ? detail.tokens / detail.events : 0} format={fmtTokens} />
      </div>
      <div class="hint">
        {detail.tokens ? ((detail.output_tokens / detail.tokens) * 100).toFixed(0) : 0}% output
      </div>
    </div>
    {#if usagePct !== null}
      <div class="card up" style="animation-delay:240ms">
        <div class="label">Share of usage</div>
        <div class="value"><AnimatedNumber value={Number(usagePct)} format={(n) => `${n.toFixed(1)}%`} /></div>
        <div class="hint">of all-time tokens</div>
      </div>
    {/if}
  </div>

  <div class="cards">
    <div class="card up">
      <div class="label">First recorded</div>
      <div class="value small">{fmtDateTime(detail.first_ts)}</div>
    </div>
    <div class="card up" style="animation-delay:60ms">
      <div class="label">Last used</div>
      <div class="value small">{fmtDateTime(detail.last_ts)}</div>
    </div>
    <div class="card up" style="animation-delay:120ms">
      <div class="label">Active days</div>
      <div class="value"><AnimatedNumber value={detail.active_days} /></div>
      <div class="hint">
        {detail.current_streak}d current streak · {detail.longest_streak}d longest
      </div>
    </div>
    {#if detail.peak_day}
      <div class="card up" style="animation-delay:180ms">
        <div class="label">Peak day</div>
        <div class="value small">{detail.peak_day}</div>
        <div class="hint">{fmtTokens(detail.peak_day_tokens)} tokens</div>
      </div>
    {/if}
  </div>

  <div class="grid2">
    <div class="panel up">
      <h2>Daily usage<span class="fig">FIG.07</span></h2>
      {#if dailyOption}
        <Chart option={dailyOption} height={260} />
      {:else}
        <div class="loading">no data</div>
      {/if}
    </div>
    <div class="panel up" style="animation-delay:80ms">
      <h2>Token mix<span class="fig">FIG.08</span></h2>
      {#if donutOption}
        <Chart option={donutOption} height={230} />
      {:else}
        <div class="loading">no data</div>
      {/if}
    </div>
  </div>

  {#if detail.by_source.length}
    <div class="panel up" style="padding:6px 14px">
      <h2 style="padding:8px 0 2px">By harness<span class="fig">FIG.09</span></h2>
      <table>
        <thead>
          <tr>
            <th>Source</th>
            <th class="num">Tokens</th>
            <th class="num">Calls</th>
            <th class="num">Sessions</th>
            <th class="num">Est. cost</th>
          </tr>
        </thead>
        <tbody>
          {#each detail.by_source as s}
            <tr>
              <td>
                <span class="stag"><i style="background:{sourceColor(s.source)}"></i>{sourceLabel(s.source)}</span>
              </td>
              <td class="num">{fmtTokens(s.tokens)}</td>
              <td class="num">{s.events.toLocaleString()}</td>
              <td class="num">{s.sessions.toLocaleString()}</td>
              <td class="num">{fmtCost(s.cost_usd)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}

  {#if detail.by_project.length}
    <div class="panel up" style="padding:6px 14px">
      <h2 style="padding:8px 0 2px">Projects worked on<span class="fig">FIG.10</span></h2>
      <table>
        <thead>
          <tr>
            <th>Project</th>
            <th class="num">Tokens</th>
            <th class="num">Calls</th>
            <th class="num">Sessions</th>
            <th class="num">Est. cost</th>
            <th class="num">First used</th>
            <th class="num">Last used</th>
          </tr>
        </thead>
        <tbody>
          {#each detail.by_project as p}
            <tr>
              <td>
                <div>{basename(p.project)}</div>
                {#if p.project !== basename(p.project)}
                  <div class="path ell">{p.project}</div>
                {/if}
              </td>
              <td class="num">{fmtTokens(p.tokens)}</td>
              <td class="num">{p.events.toLocaleString()}</td>
              <td class="num">{p.sessions.toLocaleString()}</td>
              <td class="num">{fmtCost(p.cost_usd)}</td>
              <td class="num muted">{fmtDate(p.first_ts)}</td>
              <td class="num muted">{fmtDate(p.last_ts)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
{/if}

<style>
  .backlink {
    display: inline-block;
    margin-bottom: 12px;
  }
  h1 {
    word-break: break-word;
    font-size: 30px;
    letter-spacing: 0;
  }
</style>
