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
  import { goto } from '$app/navigation';
  import { getCurrent, onOpenUrl } from '@tauri-apps/plugin-deep-link';
  import { tokenTrailDestination } from '$lib/wackchatter/navigation';
  import { listen } from '@tauri-apps/api/event';

  const links = [
    { href: '/', label: 'Overview' },
    { href: '/models', label: 'Models' },
    { href: '/families', label: 'Families' },
    { href: '/trends', label: 'Trends' },
    { href: '/projects', label: 'Projects' },
    { href: '/activity', label: 'Activity' },
    { href: '/wackcode', label: 'WackCode' },
    { href: '/wackchatter', label: 'WackChatter' },
    { href: '/settings', label: 'Settings' },
  ];

  let lastSync = $state('');
  let syncing = $state(false);

  /* ── Nav pill refs ─────────────────────────────────────────────── */
  let navEl: HTMLElement | undefined = $state();
  let linkEls: (HTMLAnchorElement | undefined)[] = $state([]);
  let pillReady = $state(false); // suppresses transition on first position

  function isActive(href: string): boolean {
    const path = $page.url.pathname;
    return href === '/' ? path === '/' : path === href || path.startsWith(href + '/');
  }

  function positionNav(ensureVisible = false) {
    if (!navEl) return;
    const idx = links.findIndex((link) => isActive(link.href));
    const el = idx >= 0 ? linkEls[idx] : undefined;
    if (!el) return;
    const x = el.offsetLeft;
    const w = el.offsetWidth;
    if (ensureVisible && (x < navEl.scrollLeft || x + w > navEl.scrollLeft + navEl.clientWidth)) {
      navEl.scrollLeft = Math.max(0, x - (navEl.clientWidth - w) / 2);
    }
    if (!pillReady) navEl.classList.add('no-animate');
    navEl.style.setProperty('--pill-x', `${x}px`);
    navEl.style.setProperty('--pill-w', `${w}px`);
    if (!pillReady) {
      void navEl.offsetHeight;
      navEl.classList.remove('no-animate');
      pillReady = true;
    }
  }

  $effect(() => {
    const pathname = $page.url.pathname;
    positionNav(true);
  });

  onMount(() => {
    // Fonts and minimum-window breakpoints can change link widths after navigation.
    const observer = new ResizeObserver(() => positionNav());
    if (navEl) observer.observe(navEl);
    for (const link of linkEls) if (link) observer.observe(link);
    return () => observer.disconnect();
  });

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

  onMount(() => {
    if ($page.url.searchParams.has('mock')) return;
    let cleanup: (() => void) | undefined;
    let disposed = false;
    const navigate = (urls:string[]) => {for (const url of urls) {const path=tokenTrailDestination(url);if(path)void goto(path);}};
    void (async () => {
      try {
        const stop = await onOpenUrl(navigate);
        if(disposed){stop();return;}cleanup=stop;
        const urls=await getCurrent();if(!disposed&&urls)navigate(urls);
      } catch { /* browser preview has no native protocol handler */ }
    })();
    return ()=>{disposed=true;cleanup?.();};
  });

  let { children } = $props();
</script>

<header class="topnav">
  <div class="mk">TokenTrail</div>
  <nav bind:this={navEl}>
    {#each links as l, i}
      <a href={l.href} class:on={isActive(l.href)} bind:this={linkEls[i]}><u>0{i + 1}</u>{l.label}</a>
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
