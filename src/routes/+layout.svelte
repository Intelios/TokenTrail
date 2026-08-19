<script lang="ts">
  import '@fontsource/anton';
  import '@fontsource/inter-tight/400.css';
  import '@fontsource/inter-tight/500.css';
  import '@fontsource/inter-tight/600.css';
  import '@fontsource/inter-tight/700.css';
  import '@fontsource/ibm-plex-mono/400.css';
  import '@fontsource/ibm-plex-mono/500.css';
  import '@fontsource/ibm-plex-mono/600.css';
  import '../app.css';
  import { onMount } from 'svelte';
  import { page } from '$app/stores';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';

  const links = [
    { href: '/', label: 'Overview' },
    { href: '/models', label: 'Models' },
    { href: '/families', label: 'Families' },
    { href: '/trends', label: 'Trends' },
    { href: '/projects', label: 'Projects' },
    { href: '/activity', label: 'Activity' },
    { href: '/settings', label: 'Settings' },
  ];

  let lastSync = $state('');
  let syncing = $state(false);

  function isActive(href: string): boolean {
    const path = $page.url.pathname;
    return href === '/' ? path === '/' : path === href || path.startsWith(href + '/');
  }

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
    lastSync = new Date().toLocaleTimeString(undefined, {
      hour: '2-digit',
      minute: '2-digit',
      hour12: false,
    });
    window.dispatchEvent(new CustomEvent('tt-sync'));
  }

  onMount(async () => {
    await listen('sync-done', () => touch());
    // the Rust side runs its first sync immediately on launch; refresh shortly after mount
    setTimeout(touch, 1500);
  });

  let { children } = $props();
</script>

<header class="topnav">
  <div class="mk">TokenTrail</div>
  <nav>
    {#each links as l, i}
      <a href={l.href} class:on={isActive(l.href)}><u>0{i + 1}</u>{l.label}</a>
    {/each}
  </nav>
  <button class="sy" onclick={doSync} disabled={syncing} title="Sync now">
    <i></i>
    {syncing ? 'Syncing…' : lastSync ? `Synced ${lastSync}` : 'Sync on launch…'}
  </button>
</header>

<main class="edge">
  {@render children()}
</main>
