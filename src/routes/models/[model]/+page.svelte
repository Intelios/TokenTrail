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
    modelColor,
    MIX_COLORS,
    REASONING_COLOR,
    sourceSwatch,
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
    const reasoning = detail.reasoning_tokens ?? 0;
    const textOutput = Math.max(0, detail.output_tokens - reasoning);
    const data = [
      { name: 'Input', value: detail.input_tokens, itemStyle: { color: MIX_COLORS[0] } },
      { name: 'Output', value: textOutput, itemStyle: { color: MIX_COLORS[1] } },
      { name: 'Reasoning', value: reasoning, itemStyle: { color: REASONING_COLOR } },
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
          itemStyle: { color: modelColor(detail.model, 0) },
          animationDelay: (idx: number) => idx * 8,
        },
      ],
    } satisfies EChartsOption;
  });
</script>

<div class="mcframe">
  {#if error}
    <div class="errpad"><p class="error">{error}</p></div>
  {:else if loading}
    <div class="loading" style="padding:40px">loading model card…</div>
  {:else if !detail}
    <div class="loading" style="padding:40px">No usage recorded for this model</div>
  {:else}
    <!-- Header -->
    <div class="thd">
      <div class="up">
        <a class="backlink" href="/models">← All models</a>
        <h1>{detail.model}</h1>
        <div class="sub">
          {detail.events.toLocaleString()} calls across
          {detail.by_source.length} harness{detail.by_source.length !== 1 ? 'es' : ''}
          {#if detail.first_ts}
            · since {fmtDate(detail.first_ts)}
          {/if}
        </div>
      </div>
    </div>

    <!-- Cards Row 1 -->
    <div class="mcards">
      <div class="mc up">
        <div class="k">Total tokens</div>
        <div class="v"><AnimatedNumber value={detail.tokens} format={fmtTokens} /></div>
        <div class="h">
          {fmtTokens(detail.input_tokens)} in · {fmtTokens(detail.output_tokens)} out{#if detail.reasoning_tokens} · {fmtTokens(detail.reasoning_tokens)} reasoning{/if}
        </div>
      </div>
      <div class="mc up" style="animation-delay:60ms">
        <div class="k">Est. cost</div>
        <div class="v"><AnimatedNumber value={detail.cost_usd ?? 0} format={fmtCost} /></div>
        <div class="h">API list price equivalent</div>
      </div>
      <div class="mc up" style="animation-delay:120ms">
        <div class="k">Calls</div>
        <div class="v"><AnimatedNumber value={detail.events} /></div>
        <div class="h">{detail.sessions.toLocaleString()} session{detail.sessions !== 1 ? 's' : ''}</div>
      </div>
      <div class="mc up" style="animation-delay:180ms">
        <div class="k">Avg tok / call</div>
        <div class="v">
          <AnimatedNumber value={detail.events ? detail.tokens / detail.events : 0} format={fmtTokens} />
        </div>
        <div class="h">{detail.tokens ? ((detail.output_tokens / detail.tokens) * 100).toFixed(0) : 0}% output</div>
      </div>
      {#if usagePct !== null}
        <div class="mc hl up" style="animation-delay:240ms">
          <div class="k">Share of usage</div>
          <div class="v"><AnimatedNumber value={Number(usagePct)} format={(n) => `${n.toFixed(1)}%`} /></div>
          <div class="h">of all-time tokens</div>
        </div>
      {/if}
    </div>

    <!-- Cards Row 2 -->
    <div class="mcards subcards">
      <div class="mc up">
        <div class="k">First recorded</div>
        <div class="v sm">{fmtDateTime(detail.first_ts)}</div>
      </div>
      <div class="mc up" style="animation-delay:60ms">
        <div class="k">Last used</div>
        <div class="v sm">{fmtDateTime(detail.last_ts)}</div>
      </div>
      <div class="mc up" style="animation-delay:120ms">
        <div class="k">Active days</div>
        <div class="v sm"><AnimatedNumber value={detail.active_days} /></div>
        <div class="h">{detail.current_streak}d streak · {detail.longest_streak}d peak</div>
      </div>
      {#if detail.peak_day}
        <div class="mc up" style="animation-delay:180ms">
          <div class="k">Peak day</div>
          <div class="v sm">{detail.peak_day}</div>
          <div class="h">{fmtTokens(detail.peak_day_tokens)} tokens</div>
        </div>
      {/if}
    </div>

    <!-- Charts -->
    <div class="mcharts">
      <div class="mplot up">
        <h2>Daily usage</h2>
        {#if dailyOption}
          <Chart option={dailyOption} height={260} />
        {:else}
          <div class="loading">no data</div>
        {/if}
      </div>
      <div class="mdonut up" style="animation-delay:80ms">
        <h2>Token mix</h2>
        {#if donutOption}
          <Chart option={donutOption} height={230} />
        {:else}
          <div class="loading">no data</div>
        {/if}
      </div>
    </div>

    <!-- By harness table -->
    {#if detail.by_source.length}
      <div class="dtable-sec up">
        <h2>By harness</h2>
        <div class="tw">
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
                    <span class="stag"><i style="background:{sourceSwatch(s.source)}"></i>{sourceLabel(s.source)}</span>
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
      </div>
    {/if}

    <!-- Projects table -->
    {#if detail.by_project.length}
      <div class="dtable-sec up">
        <h2>Projects worked on</h2>
        <div class="tw">
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
                    <div class="pname">{basename(p.project)}</div>
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
      </div>
    {/if}
  {/if}
</div>

<style>
  .mcframe {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .thd {
    padding: 16px clamp(22px, 1.8vw, 40px) 14px;
    border-bottom: 2px solid var(--ink);
  }
  .backlink {
    display: inline-block;
    font: 600 10px/1 var(--font-mono);
    letter-spacing: 1px;
    text-transform: uppercase;
    color: var(--dim);
    margin-bottom: 8px;
  }
  .backlink:hover {
    color: var(--org);
  }
  .thd h1 {
    margin: 0;
    font-size: clamp(24px, 2.2vw, 36px);
    line-height: 1.15;
    letter-spacing: -0.5px;
    word-break: break-all;
  }
  .thd .sub {
    font: 400 11px/1 var(--font-mono);
    opacity: 0.55;
    margin-top: 6px;
    letter-spacing: 0.6px;
    margin-bottom: 0;
  }

  .errpad { padding: 20px 22px; }

  .mcards {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
    border-bottom: 2px solid var(--ink);
  }
  .mc {
    padding: clamp(12px, 1.2vh, 20px) clamp(16px, 1.2vw, 24px);
    border-right: 2px solid var(--ink);
    min-width: 0;
  }
  .mc:last-child { border-right: none; }
  .mc .k { font: 600 clamp(9px, 0.7vw, 12px)/1 var(--font-ui); letter-spacing: 1.3px; text-transform: uppercase; opacity: 0.55; }
  .mc .v { font: 400 clamp(24px, 2vw, 40px)/1 var(--font-disp); margin-top: 8px; letter-spacing: -1px; font-variant-numeric: tabular-nums; }
  .mc .v.sm { font-size: clamp(14px, 1.1vw, 20px); font-family: var(--font-mono); font-weight: 600; letter-spacing: 0; margin-top: 10px; }
  .mc .h { font: 400 clamp(10px, 0.75vw, 12px)/1.3 var(--font-mono); margin-top: 7px; opacity: 0.48; }
  .mc.hl { background: var(--org); color: #fff; }
  .mc.hl .k, .mc.hl .h { opacity: 0.82; }

  .subcards {
    background: rgba(13, 13, 11, 0.02);
  }

  .mcharts {
    display: grid;
    grid-template-columns: 1.3fr 0.7fr;
    border-bottom: 2px solid var(--ink);
  }
  .mplot {
    padding: 16px clamp(22px, 1.8vw, 40px) 18px;
    border-right: 2px solid var(--ink);
  }
  .mdonut {
    padding: 16px 20px 18px;
  }
  .mplot h2, .mdonut h2, .dtable-sec h2 {
    font: 600 11px/1 var(--font-ui);
    letter-spacing: 1.6px;
    text-transform: uppercase;
    margin: 0 0 12px;
  }

  .dtable-sec {
    border-bottom: 2px solid var(--ink);
    padding: 16px 0 0;
  }
  .dtable-sec:last-child {
    border-bottom: none;
  }
  .dtable-sec h2 {
    padding: 0 clamp(22px, 1.8vw, 40px);
  }

  .tw {
    width: 100%;
    overflow-x: auto;
  }
  .tw td {
    padding: clamp(8px, 1vh, 15px) clamp(22px, 1.8vw, 40px);
  }
  .tw th {
    padding: 9px clamp(22px, 1.8vw, 40px);
  }
  .pname {
    font-weight: 600;
    font-size: 13px;
  }
  .path {
    font: 400 10.5px/1.3 var(--font-mono);
    opacity: 0.55;
    margin-top: 2px;
  }

  @media (max-width: 900px) {
    .mcards { grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); }
    .mc { border-bottom: 2px solid var(--ink); }
    .mcharts { grid-template-columns: 1fr; }
    .mplot { border-right: none; border-bottom: 2px solid var(--ink); }
  }
</style>
