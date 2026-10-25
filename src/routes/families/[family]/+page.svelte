<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import type { EChartsOption } from 'echarts';
  import Chart from '$lib/Chart.svelte';
  import AnimatedNumber from '$lib/AnimatedNumber.svelte';
  import AchievementIcon from '$lib/AchievementIcon.svelte';
  import Spark from '$lib/Spark.svelte';
  import { api, type Achievement, type DailyModelRow, type FamilyDetail } from '$lib/api';
  import {
    fmtCost,
    fmtDate,
    fmtDateTime,
    fmtTokens,
    familyColor,
    familySwatch,
    familyFlat,
    MODEL_PALETTE,
    sourceSwatch,
    sourceLabel,
    basename,
    achColor,
    markedColors,
  } from '$lib/format';
  import { TOOLTIP, ANIM, donutSeries } from '$lib/chartTheme';
  import { singleDailyColumns, singleDailyOption } from '$lib/dailyColumns';
  import { readPref, writePref } from '$lib/prefs';

  const RANGES: [number, string][] = [
    [7, '7D'],
    [30, '30D'],
    [90, '90D'],
    [3650, 'ALL'],
  ];

  const RANGE_LABEL: Record<number, string> = {
    7: '7-day',
    30: '30-day',
    90: '90-day',
    3650: 'all-time',
  };

  // Shared with the Families list, so the range carries between the two.
  const PREF_DAYS = 'tt.families.days';

  let days = $state(readPref(PREF_DAYS, 90, (v) => RANGES.some(([d]) => d === v)));
  let detail = $state<FamilyDetail | null>(null);
  let achievements = $state<Achievement[]>([]);
  let trendRows = $state<DailyModelRow[]>([]);
  let projectColors = $state<Record<string, string>>({});
  let error = $state('');
  let loading = $state(true);

  const name = $derived($page.params.family ?? '');

  async function load() {
    if (!name) return;
    if (!detail) loading = true;
    try {
      const [d, achs, trends, colors] = await Promise.all([
        api.familyDetail(name, days),
        api.familyAchievements(name),
        api.dailyByModel(90),
        api.projectColors(),
      ]);
      detail = d;
      achievements = achs;
      trendRows = trends;
      projectColors = markedColors(colors);
      error = '';
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    name;
    days;
    untrack(() => {
      // must not be reactive dependencies: load() assigns a fresh `detail`
      // object every call, which would re-run this effect forever
      if (detail && detail.family.toLowerCase() !== name.toLowerCase()) {
        detail = null;
        achievements = [];
      }
      load();
    });
  });

  $effect(() => {
    writePref(PREF_DAYS, days);
  });

  onMount(() => {
    const h = () => load();
    window.addEventListener('tt-sync', h);
    return () => window.removeEventListener('tt-sync', h);
  });

  const usagePct = $derived.by(() => {
    if (!detail || !detail.total_window_tokens) return null;
    return ((detail.tokens / detail.total_window_tokens) * 100).toFixed(1);
  });

  // models arrive sorted by tokens, so the leader is the first row
  const topModel = $derived(detail?.models[0] ?? null);

  const dayCols = $derived(singleDailyColumns(detail?.daily ?? []));
  const dailyOption = $derived(
    detail ? singleDailyOption(dayCols, familyColor(detail.family, 0)) : undefined,
  );

  // Member models share one brand colour through familyColor(), so the donut
  // cycles the model palette instead — a legible slice per member.
  const modelMixOption = $derived.by(() => {
    if (!detail || !detail.models.length) return undefined;
    const data = detail.models
      .map((m, i) => ({
        name: m.model,
        value: m.tokens,
        itemStyle: { color: MODEL_PALETTE[i % MODEL_PALETTE.length] },
      }))
      .filter((d) => d.value > 0);
    if (!data.length) return undefined;
    return {
      backgroundColor: 'transparent',
      ...ANIM,
      tooltip: {
        ...TOOLTIP,
        formatter: (p: any) =>
          `${p.name}<br/>${fmtTokens(Number(p.value ?? 0))} (${p.percent}%)`,
      },
      series: [donutSeries(data)],
    } satisfies EChartsOption;
  });

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
      out.set(
        model,
        [...m.entries()].sort((a, b) => (a[0] < b[0] ? -1 : 1)).map(([, v]) => v),
      );
    }
    return out;
  });

  function modelUrl(model: string): string {
    return '/models/' + encodeURIComponent(model);
  }

  function projectUrl(project: string): string {
    return '/projects/' + encodeURIComponent(project);
  }

  function rowClick(e: MouseEvent, href: string) {
    if ((e.target as HTMLElement | null)?.closest('a')) return;
    goto(href);
  }
</script>

<div class="mcframe">
  {#if error}
    <div class="errpad"><p class="error">{error}</p></div>
  {:else if loading}
    <div class="loading" style="padding:40px">loading family card…</div>
  {:else if !detail}
    <div class="thd">
      <div class="up">
        <a class="backlink" href="/families">← All families</a>
        <h1>{name}</h1>
        <div class="sub">No usage recorded for this family</div>
      </div>
    </div>
  {:else}
    <!-- Header -->
    <div class="thd">
      <div class="up">
        <a class="backlink" href="/families">← All families</a>
        <div class="titlerow">
          <span class="famswatch" style="background:{familySwatch(detail.family, 0)}"></span>
          <h1>{detail.family}</h1>
        </div>
        <div class="sub">
          {detail.events.toLocaleString()} calls across {detail.models.length} model{detail.models.length !== 1
            ? 's'
            : ''}
          · {detail.by_source.length} harness{detail.by_source.length !== 1 ? 'es' : ''}
          {#if detail.first_ts}
            · since {fmtDate(detail.first_ts)}
          {/if}
        </div>
      </div>
      <div class="pills up">
        {#each RANGES as [d, label]}
          <button class="pill" class:on={days === d} onclick={() => (days = d)}>{label}</button>
        {/each}
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
        <div class="k">Avg tok / day</div>
        <div class="v">
          <AnimatedNumber
            value={detail.active_days ? detail.tokens / detail.active_days : 0}
            format={fmtTokens}
          />
        </div>
        <div class="h">{detail.active_days} active day{detail.active_days !== 1 ? 's' : ''}</div>
      </div>
      {#if usagePct !== null}
        <div class="mc hl up" style="animation-delay:240ms">
          <div class="k">Share of usage</div>
          <div class="v"><AnimatedNumber value={Number(usagePct)} format={(n) => `${n.toFixed(1)}%`} /></div>
          <div class="h">of {RANGE_LABEL[days] ?? `${days}-day`} tokens</div>
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
      {#if topModel}
        <div class="mc up" style="animation-delay:180ms">
          <div class="k">Top model</div>
          <div class="v sm ell">
            <a class="toplink" href={modelUrl(topModel.model)} title={topModel.model}>{topModel.model}</a>
          </div>
          <div class="h">
            {detail.tokens ? ((topModel.tokens / detail.tokens) * 100).toFixed(1) : 0}% of family tokens
          </div>
        </div>
      {/if}
      {#if detail.peak_day}
        <div class="mc up" style="animation-delay:240ms">
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
          <div class="loading">no data in this window</div>
        {/if}
      </div>
      <div class="mdonut up" style="animation-delay:80ms">
        <h2>Model mix</h2>
        {#if modelMixOption}
          <Chart option={modelMixOption} height={230} />
        {:else}
          <div class="loading">no data in this window</div>
        {/if}
      </div>
    </div>

    <!-- Member models -->
    {#if detail.models.length}
      <div class="dtable-sec up">
        <h2>Models in family ({detail.models.length})</h2>
        <div class="tw dense">
          <table>
            <thead>
              <tr>
                <th>#</th>
                <th>Model</th>
                <th>Sources</th>
                <th class="num">Calls</th>
                <th class="num">Tokens</th>
                <th class="num">In / Out</th>
                <th class="num">Avg/day</th>
                <th>90d trend</th>
                <th class="num">Est. cost</th>
                <th class="num">Share</th>
              </tr>
            </thead>
            <tbody>
              {#each detail.models as m, mi}
                {@const trend = trendByModel.get(m.model)}
                {@const share = detail.tokens ? (m.tokens / detail.tokens) * 100 : 0}
                <tr
                  class="up clickrow"
                  style="animation-delay:{mi * 40}ms"
                  onclick={(e) => rowClick(e, modelUrl(m.model))}
                >
                  <td class="rk">
                    <span class="chip" style="background:{familySwatch(detail.family, 0)};opacity:0.65">{mi + 1}</span>
                  </td>
                  <td>
                    <a class="modellink" href={modelUrl(m.model)} title={m.model}>{m.model}</a>
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
                  <td class="num">{m.active_days ? fmtTokens(m.tokens / m.active_days) : '—'}</td>
                  <td>
                    {#if trend && trend.length > 1}
                      <Spark
                        values={trend}
                        width={94}
                        height={20}
                        color={familyFlat(detail.family, 0)}
                        delay={200 + mi * 40}
                      />
                    {:else}
                      <span class="dim">—</span>
                    {/if}
                  </td>
                  <td class="num">{fmtCost(m.cost_usd)}</td>
                  <td class="num">
                    <span class="shbar">
                      <div
                        class="gw"
                        style="width:{Math.max(2, Math.min(100, share))}%;background:{familySwatch(detail.family, 0)};animation-delay:{mi * 40}ms"
                      ></div>
                    </span>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      </div>
    {/if}

    <!-- By harness -->
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

    <!-- Projects -->
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
                <tr class="clickrow" onclick={(e) => rowClick(e, projectUrl(p.project))}>
                  <td>
                    <div class="pname">
                      {#if projectColors[p.project]}
                        <span class="pchip" style="background:{projectColors[p.project]}"></span>
                      {/if}
                      <a class="projlink" href={projectUrl(p.project)}>{basename(p.project)}</a>
                    </div>
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

    <!-- Achievements -->
    {#if achievements.length}
      <div class="dtable-sec up">
        <h2>Achievements</h2>
        <div class="ach-grid">
          {#each achievements as a, i}
            <div
              class="ach-card up"
              style="border-left-color: {achColor(a)}; animation-delay: {Math.min(i * 40, 400)}ms"
            >
              <div class="ach-head">
                <span class="ach-icon-wrap" style="color: {achColor(a)}">
                  <AchievementIcon kind={a.kind} size={15} />
                </span>
                <span class="ach-title">{a.title}</span>
              </div>
              <div class="ach-val" title={a.value}>
                {a.kind === 'first_project' ? basename(a.value) : a.value}
              </div>
              <div class="ach-date">
                {#if a.earned_ts}
                  Earned {fmtDate(a.earned_ts)}
                {:else}
                  Current standing
                {/if}
              </div>
            </div>
          {/each}
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
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 16px;
    padding: 16px clamp(22px, 1.8vw, 40px) 14px;
    border-bottom: 2px solid var(--ink);
    flex-wrap: wrap;
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
  .titlerow {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .famswatch {
    width: 15px;
    height: 15px;
    flex: none;
    border: 2px solid var(--ink);
  }
  .thd h1 {
    margin: 0;
    font-size: clamp(24px, 2.2vw, 36px);
    line-height: 1.15;
    letter-spacing: -0.5px;
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

  /* the 10-column member table needs narrower gutters than the detail tables */
  .tw.dense td {
    padding: clamp(7px, 0.9vh, 14px) 12px;
  }
  .tw.dense th {
    padding: 9px 12px;
  }
  .tw.dense td:first-child,
  .tw.dense th:first-child {
    padding-left: clamp(22px, 1.8vw, 40px);
  }
  .tw.dense td:last-child,
  .tw.dense th:last-child {
    padding-right: clamp(22px, 1.8vw, 40px);
  }

  .clickrow {
    cursor: pointer;
  }
  .clickrow:hover td {
    background: rgba(13, 13, 11, 0.035);
  }

  .rk {
    width: 34px;
    text-align: center;
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
  .toplink {
    color: inherit;
    text-decoration: none;
  }
  .toplink:hover {
    color: var(--org);
    text-decoration: underline;
  }

  .pname {
    font-weight: 600;
    font-size: 13px;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .projlink {
    color: inherit;
    text-decoration: none;
  }
  .projlink:hover {
    color: var(--org);
    text-decoration: underline;
  }
  .pchip {
    display: inline-block;
    width: 8px;
    height: 8px;
    flex: none;
    border: 1px solid var(--ink);
  }
  .path {
    font: 400 10.5px/1.3 var(--font-mono);
    opacity: 0.55;
    margin-top: 2px;
  }
  .muted {
    opacity: 0.6;
  }

  @media (max-width: 900px) {
    .mcards { grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); }
    .mc { border-bottom: 2px solid var(--ink); }
    .mcharts { grid-template-columns: 1fr; }
    .mplot { border-right: none; border-bottom: 2px solid var(--ink); }
  }

  .ach-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 12px;
    padding: 0 clamp(22px, 1.8vw, 40px) 24px;
  }
  .ach-card {
    background: rgba(13, 13, 11, 0.02);
    border: 2px solid var(--ink);
    border-left-width: 5px;
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .ach-head {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .ach-icon-wrap {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }
  .ach-title {
    font: 600 10px/1 var(--font-ui);
    letter-spacing: 1px;
    text-transform: uppercase;
    opacity: 0.6;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ach-val {
    font: 600 15px/1.2 var(--font-ui);
    letter-spacing: -0.2px;
    word-break: break-word;
  }
  .ach-date {
    font: 400 11px/1.2 var(--font-mono);
    opacity: 0.5;
    margin-top: 2px;
  }
</style>
