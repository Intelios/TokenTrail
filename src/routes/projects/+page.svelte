<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type ProjectRow } from '$lib/api';
  import { basename, fmtCost, fmtDate, fmtTokens } from '$lib/format';

  const RANGES: [number, string][] = [
    [30, '30 days'],
    [90, '90 days'],
    [3650, 'All time'],
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

<h1>Projects</h1>
<p class="sub">Where your tokens actually go — ranked by lifetime usage</p>

<div class="row" style="margin-bottom:16px">
  {#each RANGES as [d, label]}
    <button class="pill" class:on={days === d} onclick={() => (days = d)}>{label}</button>
  {/each}
</div>

{#if error}
  <p class="error">{error}</p>
{:else if !projects.length}
  <div class="loading">no projects recorded yet</div>
{:else}
  <div class="panel" style="padding:6px 12px">
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
        {#each projects as p}
          <tr>
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
