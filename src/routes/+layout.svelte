<script lang="ts">
  import '../app.css';
  import { onMount } from 'svelte';
  import { page } from '$app/stores';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';

  const links = [
    { href: '/', label: 'Overview' },
    { href: '/models', label: 'Models' },
    { href: '/trends', label: 'Trends' },
    { href: '/projects', label: 'Projects' },
    { href: '/activity', label: 'Activity' },
    { href: '/settings', label: 'Settings' },
  ];

  let lastSync = $state('');
  let syncing = $state(false);

  async function doSync() {
    syncing = true;
    try {
      await invoke('sync_now');
      touch();
    } finally {
      syncing = false;
    }
  }

  function touch() {
    lastSync = new Date().toLocaleTimeString();
    window.dispatchEvent(new CustomEvent('tt-sync'));
  }

  onMount(async () => {
    await listen('sync-done', () => touch());
    // the Rust side runs its first sync immediately on launch; refresh shortly after mount
    setTimeout(touch, 1500);
  });

  let { children } = $props();
</script>

<div class="shell">
  <aside>
    <div class="brand"><span class="dot"></span><span>TokenTrail</span></div>
    <nav>
      {#each links as l}
        <a href={l.href} class:active={$page.url.pathname === l.href}>{l.label}</a>
      {/each}
    </nav>
    <div class="syncbar">
      <button class="primary" onclick={doSync} disabled={syncing}>{syncing ? 'Syncing…' : 'Sync now'}</button>
      {#if lastSync}
        <div>last sync {lastSync}</div>
      {:else}
        <div>syncing on launch…</div>
      {/if}
    </div>
  </aside>
  <main>
    {@render children()}
  </main>
</div>
