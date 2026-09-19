<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import Chart from '$lib/Chart.svelte';
  import { api, type DailyProjectRow, type ProjectRow } from '$lib/api';
  import { basename, fmtCost, fmtDate, fmtTokens } from '$lib/format';
  import {
    projectDailyColumns,
    projectDailyOption,
    dailyRange,
    spineTotals,
  } from '$lib/dailyColumns';
  import { readPref, writePref } from '$lib/prefs';

  const RANGES: [number, string][] = [
    [7, '7D'],
    [30, '30D'],
    [90, '90D'],
    [3650, 'ALL'],
  ];
  const PREF_DAYS = 'tt.projects.days';

  let days = $state(readPref(PREF_DAYS, 3650, (v) => RANGES.some(([d]) => d === v)));
  let projects = $state<ProjectRow[]>([]);
  let dailyProjectRows = $state<DailyProjectRow[]>([]);
  let error = $state('');

  function projectUrl(p: string): string {
    return '/projects/' + encodeURIComponent(p);
  }

  async function load() {
    try {
      const [p, d] = await Promise.all([api.byProject(days), api.dailyByProject(days)]);
      projects = p;
      dailyProjectRows = d;
      error = '';
    } catch (e) {
      error = String(e);
    }
  }

  $effect(() => {
    days;
    load();
  });

  $effect(() => {
    writePref(PREF_DAYS, days);
  });

  onMount(() => {
    const h = () => load();
    window.addEventListener('tt-sync', h);
    return () => window.removeEventListener('tt-sync', h);
  });

  // ── daily stacked columns ──
  const dailyProjects = $derived.by(() => {
    const totals = new Map<string, number>();
    for (const r of dailyProjectRows) {
      totals.set(r.project, (totals.get(r.project) ?? 0) + r.tokens);
    }
    return [...totals.entries()]
      .sort((a, b) => b[1] - a[1])
      .map(([p]) => p);
  });

  const dayCols = $derived(projectDailyColumns(dailyProjectRows, dailyProjects));
  const dayOption = $derived(projectDailyOption(dayCols, dailyProjects));
  const dayRangeStr = $derived(dailyRange(dayCols));

  // second half of the charted calendar vs the first half
  const deltaPct = $derived.by(() => {
    const dList = spineTotals(dayCols);
    if (dList.length < 8) return null;
    const mid = Math.floor(dList.length / 2);
    const first = dList.slice(0, mid).reduce((a, b) => a + b, 0);
    const second = dList.slice(mid).reduce((a, b) => a + b, 0);
    if (first === 0) return null;
    return ((second - first) / first) * 100;
  });
</script>

<div class="pframe">
  <div class="thd">
    <div class="up">
      <h1>Projects</h1>
      <div class="sub">Where your tokens actually go — ranked by usage · {projects.length} tracked</div>
    </div>
    <div class="pills up">
      {#each RANGES as [d, label]}
        <button class="pill" class:on={days === d} onclick={() => (days = d)}>{label}</button>
      {/each}
    </div>
  </div>

  {#if error}
    <div class="errpad"><p class="error">{error}</p></div>
  {:else if !projects.length}
    <div class="loading">no projects recorded yet</div>
  {:else}
    <!-- daily stacked columns -->
    {#if dailyProjectRows.length > 0 && dayOption}
      <section class="daily">
        <div class="hd">
          <h3>Daily tokens by project{dayRangeStr ? ` — ${dayRangeStr}` : ''}</h3>
          <div class="rt">
            {#if deltaPct !== null}
              <b>{deltaPct >= 0 ? '▲' : '▼'} {Math.abs(deltaPct).toFixed(0)}% VS PREV</b>
            {/if}
          </div>
        </div>
        <div class="plot">
          <Chart option={dayOption} height="fill" />
        </div>
      </section>
    {/if}

    <div class="tw">
      <table>
        <thead>
          <tr>
            <th>Project</th>
            <th class="num">Sessions</th>
            <th class="num">Model calls</th>
            <th class="num">Tokens</th>
            <th class="num">Est. cost</th>
            <th class="num">Last active</th>
          </tr>
        </thead>
        <tbody>
          {#each projects as p, i}
            <tr
              class="up projrow"
              style="animation-delay:{Math.min(i, 14) * 35}ms; cursor:pointer"
              onclick={() => goto(projectUrl(p.project))}
            >
              <td>
                <a
                  class="projectlink"
                  href={projectUrl(p.project)}
                  onclick={(e) => e.stopPropagation()}
                >
                  {p.project === 'unknown' ? 'Unknown' : basename(p.project)}
                </a>
                {#if p.project !== 'unknown'}
                  <div class="path">{p.project}</div>
                {/if}
              </td>
              <td class="num">{p.sessions.toLocaleString()}</td>
              <td class="num">{p.events.toLocaleString()}</td>
              <td class="num">{fmtTokens(p.tokens)}</td>
              <td class="num">{fmtCost(p.cost_usd)}</td>
              <td class="num muted">{fmtDate(p.last_ts)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</div>

<style>
  .pframe {
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

  .daily {
    display: flex;
    flex-direction: column;
    padding: clamp(13px, 1.3vh, 22px) clamp(22px, 1.8vw, 40px) clamp(10px, 1vh, 16px);
    border-bottom: 2px solid var(--ink);
    height: clamp(240px, 28vh, 320px);
    flex: none;
  }
  .daily .hd { display: flex; justify-content: space-between; align-items: baseline; flex: none; }
  .daily .hd h3 { font: 600 clamp(11px, 0.85vw, 15px)/1 var(--font-ui); letter-spacing: 1.6px; text-transform: uppercase; margin: 0; }
  .daily .hd .rt { display: flex; gap: 14px; align-items: baseline; }
  .daily .hd .rt b { font: 500 clamp(10px, 0.75vw, 13px)/1 var(--font-mono); letter-spacing: 1px; color: var(--org); }
  .daily .plot {
    flex: 1;
    min-height: 0;
    margin-top: 12px;
    display: flex;
    flex-direction: column;
  }

  .errpad { padding: 20px 22px; }

  .tw {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .tw td {
    padding: clamp(8px, 1vh, 17px) clamp(22px, 1.8vw, 40px);
  }
  .tw th {
    padding: 9px clamp(22px, 1.8vw, 40px);
  }
  .projrow:hover {
    background: rgba(13, 13, 11, 0.03);
  }
  .projectlink {
    font-weight: 600;
    font-size: 13.5px;
    color: var(--ink);
    text-decoration: none;
  }
  .projectlink:hover {
    color: var(--org);
  }
  .path {
    font: 400 10.5px/1.3 var(--font-mono);
    opacity: 0.55;
    margin-top: 2px;
  }
</style>
