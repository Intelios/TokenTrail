<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type ProjectRow } from '$lib/api';
  import { basename, fmtCost, fmtDate, fmtTokens } from '$lib/format';

  const RANGES: [number, string][] = [
    [30, '30D'],
    [90, '90D'],
    [3650, 'ALL'],
  ];
  let days = $state(3650);
  let projects = $state<ProjectRow[]>([]);
  let error = $state('');

  async function load() {
    try {
      projects = await api.byProject(days);
      error = '';
    } catch (e) {
      error = String(e);
    }
  }

  $effect(() => {
    days;
    load();
  });

  onMount(() => {
    const h = () => load();
    window.addEventListener('tt-sync', h);
    return () => window.removeEventListener('tt-sync', h);
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
            <tr class="up" style="animation-delay:{Math.min(i, 14) * 35}ms">
              <td>
                <div>{p.project === 'unknown' ? 'Unknown' : basename(p.project)}</div>
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
</style>
