<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { page } from '$app/stores';
  import { openPath, revealItemInDir } from '@tauri-apps/plugin-opener';
  import type { EChartsOption } from 'echarts';
  import Chart from '$lib/Chart.svelte';
  import AnimatedNumber from '$lib/AnimatedNumber.svelte';
  import { api, type ProjectDetail } from '$lib/api';
  import {
    fmtCost,
    fmtDate,
    fmtDateTime,
    fmtDuration,
    fmtTokens,
    modelFlat,
    modelSwatch,
    sourceSwatch,
    sourceLabel,
    basename,
  } from '$lib/format';
  import { TOOLTIP, ANIM, AXIS_LABEL, AXIS_LINE, SPLIT_LINE, donutSeries, dateTick } from '$lib/chartTheme';
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

  const PREF_DAYS = 'tt.projects.days';

  let days = $state(readPref(PREF_DAYS, 90, (v) => RANGES.some(([d]) => d === v)));
  let detail = $state<ProjectDetail | null>(null);
  let error = $state('');
  let loading = $state(true);
  let showAllSessions = $state(false);
  let copiedPath = $state(false);
  let copiedSessionId = $state<string | null>(null);

  const rawParam = $derived($page.params.project ?? '');
  const projectName = $derived.by(() => {
    if (!rawParam) return '';
    try {
      return decodeURIComponent(rawParam);
    } catch {
      return rawParam;
    }
  });

  async function load() {
    if (!projectName) return;
    if (!detail) loading = true;
    try {
      let d = await api.projectDetail(projectName, days);
      if (!d && !projectName.startsWith('/') && projectName !== 'unknown') {
        d = await api.projectDetail('/' + projectName, days);
      }
      detail = d;
      error = '';
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    projectName;
    days;
    untrack(() => {
      // must not be reactive dependencies: load() assigns a fresh `detail`
      // object every call, which would re-run this effect forever
      if (detail && detail.project !== projectName && detail.project !== '/' + projectName) {
        detail = null;
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

  async function handleReveal() {
    if (!detail || detail.project === 'unknown') return;
    try {
      await openPath(detail.project);
    } catch {
      try {
        await revealItemInDir(detail.project);
      } catch (e) {
        console.error('Failed to reveal project in files:', e);
      }
    }
  }

  async function handleCopyPath() {
    if (!detail || detail.project === 'unknown') return;
    try {
      await navigator.clipboard.writeText(detail.project);
      copiedPath = true;
      setTimeout(() => (copiedPath = false), 2000);
    } catch (e) {
      console.error('Failed to copy path:', e);
    }
  }

  async function handleCopySession(sid: string) {
    try {
      await navigator.clipboard.writeText(sid);
      copiedSessionId = sid;
      setTimeout(() => {
        if (copiedSessionId === sid) copiedSessionId = null;
      }, 2000);
    } catch (e) {
      console.error('Failed to copy session id:', e);
    }
  }

  function shortSessionId(sid: string): string {
    if (sid.length <= 16) return sid;
    return sid.slice(0, 8) + '…' + sid.slice(-6);
  }

  const usagePct = $derived.by(() => {
    if (!detail || !detail.total_window_tokens) return null;
    return ((detail.tokens / detail.total_window_tokens) * 100).toFixed(1);
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
          itemStyle: { color: '#ff6b35' },
          animationDelay: (idx: number) => idx * 8,
        },
      ],
    } satisfies EChartsOption;
  });

  const modelMixOption = $derived.by(() => {
    if (!detail || !detail.by_model.length) return undefined;
    const data = detail.by_model
      .map((m, i) => ({
        name: m.model,
        value: m.tokens,
        itemStyle: { color: modelFlat(m.model, i) },
      }))
      .filter((d) => d.value > 0);
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

  const displayedSessions = $derived.by(() => {
    if (!detail) return [];
    return showAllSessions ? detail.sessions_list : detail.sessions_list.slice(0, 50);
  });
</script>

<div class="pcframe">
  {#if error}
    <div class="errpad"><p class="error">{error}</p></div>
  {:else if loading}
    <div class="loading" style="padding:40px">loading project details…</div>
  {:else if !detail}
    <div class="thd">
      <div class="up">
        <a class="backlink" href="/projects">← All projects</a>
        <h1>{projectName === 'unknown' ? 'Unknown project' : basename(projectName)}</h1>
        <div class="sub">No usage recorded for this project</div>
      </div>
    </div>
  {:else}
    <!-- Header -->
    <div class="thd">
      <div class="up">
        <a class="backlink" href="/projects">← All projects</a>
        <h1>{detail.project === 'unknown' ? 'Unknown project' : basename(detail.project)}</h1>

        {#if detail.project !== 'unknown'}
          <div class="path-bar">
            <span class="path-text" title={detail.project}>{detail.project}</span>
            <div class="path-actions">
              <button class="paction" onclick={handleReveal} title="Reveal directory in system file manager">
                Reveal in Files
              </button>
              <button class="paction" onclick={handleCopyPath} title="Copy path to clipboard">
                {copiedPath ? '✓ Copied' : 'Copy path'}
              </button>
            </div>
          </div>
        {:else}
          <div class="unknown-notice">
            Events with no detected directory or unassigned project context
          </div>
        {/if}

        <div class="sub">
          {detail.events.toLocaleString()} calls across {detail.sessions.toLocaleString()} sessions
          · {detail.by_source.length} harness{detail.by_source.length !== 1 ? 'es' : ''}
          · {detail.by_model.length} model{detail.by_model.length !== 1 ? 's' : ''}
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
        <div class="k">Sessions</div>
        <div class="v"><AnimatedNumber value={detail.sessions} /></div>
        <div class="h">
          {detail.sessions ? (detail.events / detail.sessions).toFixed(1) : '0'} calls / session
        </div>
      </div>
      <div class="mc up" style="animation-delay:180ms">
        <div class="k">Model calls</div>
        <div class="v"><AnimatedNumber value={detail.events} /></div>
        <div class="h">
          {detail.events ? fmtTokens(detail.tokens / detail.events) : '—'} avg tok / call
        </div>
      </div>
      {#if usagePct !== null}
        <div class="mc hl up" style="animation-delay:240ms">
          <div class="k">Share of usage</div>
          <div class="v"><AnimatedNumber value={Number(usagePct)} format={(n) => `${n.toFixed(1)}%`} /></div>
          <div class="h">of {RANGE_LABEL[days] ?? `${days}-day`} all projects</div>
        </div>
      {/if}
    </div>

    <!-- Cards Row 2 (Subcards) -->
    <div class="mcards subcards">
      <div class="mc up">
        <div class="k">First recorded</div>
        <div class="v sm">{fmtDateTime(detail.first_ts)}</div>
      </div>
      <div class="mc up" style="animation-delay:60ms">
        <div class="k">Last active</div>
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
        <h2>Daily activity</h2>
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

    <!-- Breakdown Table: Models used -->
    {#if detail.by_model.length}
      <div class="dtable-sec up">
        <h2>Models used ({detail.by_model.length})</h2>
        <div class="tw">
          <table>
            <thead>
              <tr>
                <th>Model</th>
                <th class="num">Calls</th>
                <th class="num">Tokens</th>
                <th class="num">In / Out</th>
                <th class="num">Est. cost</th>
                <th class="num">First active</th>
                <th class="num">Last active</th>
              </tr>
            </thead>
            <tbody>
              {#each detail.by_model as m, i}
                <tr>
                  <td>
                    <a
                      class="modellink"
                      href={'/models/' + encodeURIComponent(m.model)}
                    >
                      <span class="mchip" style="background:{modelSwatch(m.model, i)}"></span>
                      {m.model}
                    </a>
                  </td>
                  <td class="num">{m.events.toLocaleString()}</td>
                  <td class="num font-bold">{fmtTokens(m.tokens)}</td>
                  <td class="num inout">
                    {fmtTokens(m.input_tokens)} in<br />{fmtTokens(m.output_tokens)} out
                  </td>
                  <td class="num">{fmtCost(m.cost_usd)}</td>
                  <td class="num muted">{fmtDate(m.first_ts)}</td>
                  <td class="num muted">{fmtDate(m.last_ts)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      </div>
    {/if}

    <!-- Breakdown Table: By harness -->
    {#if detail.by_source.length}
      <div class="dtable-sec up">
        <h2>By harness ({detail.by_source.length})</h2>
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

    <!-- Breakdown Table: Sessions list -->
    {#if detail.sessions_list.length}
      <div class="dtable-sec up">
        <div class="sec-hd">
          <h2>Sessions ({detail.sessions_list.length})</h2>
          {#if detail.sessions_list.length > 50}
            <span class="sec-note">Showing {displayedSessions.length} of {detail.sessions_list.length}</span>
          {/if}
        </div>
        <div class="tw">
          <table>
            <thead>
              <tr>
                <th>Session ID</th>
                <th>Harness</th>
                <th>Models</th>
                <th class="num">Calls</th>
                <th class="num">Tokens</th>
                <th class="num">Est. cost</th>
                <th class="num">Started</th>
                <th class="num">Duration</th>
              </tr>
            </thead>
            <tbody>
              {#each displayedSessions as s}
                <tr>
                  <td>
                    <div class="sid-wrap">
                      <span class="sid" title={s.session_id}>{shortSessionId(s.session_id)}</span>
                      <button
                        class="copy-btn"
                        onclick={() => handleCopySession(s.session_id)}
                        title="Copy full session ID"
                      >
                        {copiedSessionId === s.session_id ? '✓' : '⧉'}
                      </button>
                    </div>
                  </td>
                  <td>
                    <span class="stag"><i style="background:{sourceSwatch(s.source)}"></i>{sourceLabel(s.source)}</span>
                  </td>
                  <td>
                    <div class="mchips">
                      {#each s.models as sm}
                        <span class="modtag">{sm}</span>
                      {/each}
                    </div>
                  </td>
                  <td class="num">{s.events.toLocaleString()}</td>
                  <td class="num">{fmtTokens(s.tokens)}</td>
                  <td class="num">{fmtCost(s.cost_usd)}</td>
                  <td class="num muted">{fmtDateTime(s.first_ts)}</td>
                  <td class="num muted">{fmtDuration(s.last_ts - s.first_ts)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>

        {#if detail.sessions_list.length > 50}
          <div class="expand-bar">
            <button class="pill on" onclick={() => (showAllSessions = !showAllSessions)}>
              {showAllSessions ? 'Show less (first 50)' : `Show all ${detail.sessions_list.length} sessions`}
            </button>
          </div>
        {/if}
      </div>
    {/if}
  {/if}
</div>

<style>
  .pcframe {
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
    text-decoration: none;
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

  .path-bar {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 6px;
    flex-wrap: wrap;
  }
  .path-text {
    font: 400 11.5px/1.4 var(--font-mono);
    opacity: 0.65;
    background: rgba(13, 13, 11, 0.04);
    padding: 3px 8px;
    border: 1px solid rgba(13, 13, 11, 0.15);
    word-break: break-all;
  }
  .path-actions {
    display: flex;
    gap: 6px;
  }
  .paction {
    background: transparent;
    border: 1px solid var(--ink);
    padding: 3px 8px;
    font: 600 10px/1 var(--font-mono);
    letter-spacing: 0.5px;
    text-transform: uppercase;
    cursor: pointer;
    color: var(--ink);
  }
  .paction:hover {
    background: var(--ink);
    color: var(--bone);
  }

  .unknown-notice {
    font: 400 11px/1.4 var(--font-mono);
    opacity: 0.6;
    margin-top: 6px;
  }

  .thd .sub {
    font: 400 11px/1 var(--font-mono);
    opacity: 0.55;
    margin-top: 8px;
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
  .sec-hd {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding-right: clamp(22px, 1.8vw, 40px);
  }
  .sec-note {
    font: 400 10.5px/1 var(--font-mono);
    opacity: 0.5;
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

  .modellink {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-weight: 600;
    font-size: 13px;
    color: var(--ink);
    text-decoration: none;
  }
  .modellink:hover {
    color: var(--org);
  }
  .mchip {
    display: inline-block;
    width: 8px;
    height: 8px;
    flex-shrink: 0;
  }

  .inout {
    font-size: 11px;
    line-height: 1.35;
    opacity: 0.8;
  }

  .sid-wrap {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .sid {
    font: 600 11.5px/1 var(--font-mono);
    color: var(--ink);
    letter-spacing: 0.2px;
  }
  .copy-btn {
    background: transparent;
    border: none;
    font-size: 11px;
    cursor: pointer;
    color: var(--dim);
    padding: 1px 4px;
  }
  .copy-btn:hover {
    color: var(--org);
  }

  .mchips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .modtag {
    font: 500 10px/1.2 var(--font-mono);
    background: rgba(13, 13, 11, 0.05);
    padding: 2px 5px;
    border: 1px solid rgba(13, 13, 11, 0.1);
  }

  .expand-bar {
    display: flex;
    justify-content: center;
    padding: 14px 20px 20px;
    border-top: 1px solid rgba(13, 13, 11, 0.08);
  }

  @media (max-width: 900px) {
    .mcards { grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); }
    .mc { border-bottom: 2px solid var(--ink); }
    .mcharts { grid-template-columns: 1fr; }
    .mplot { border-right: none; border-bottom: 2px solid var(--ink); }
  }
</style>
