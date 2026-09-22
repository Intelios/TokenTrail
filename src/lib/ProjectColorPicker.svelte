<script lang="ts">
  import { api } from './api';
  import { MARK_PALETTE } from './format';

  /// Square swatch that pins a project's chart color. A pinned color shows
  /// solid; an auto (rank palette) color shows hatched, so it's clear the color
  /// will move with the ranking. Saves through the backend itself, then hands
  /// the new value to `onchange` so the page can update its own map.
  let {
    project,
    color,
    auto,
    size = 14,
    onchange,
  }: {
    project: string;
    color: string | null;
    auto?: string;
    size?: number;
    onchange: (color: string | null) => void;
  } = $props();

  let open = $state(false);
  let busy = $state(false);
  let error = $state('');
  let trigger: HTMLButtonElement | undefined = $state();
  let pop: HTMLDivElement | undefined = $state();
  let pos = $state({ left: 0, top: 0 });

  const isHex = (c: string | undefined): c is string => !!c && /^#[0-9a-f]{6}$/i.test(c);
  const customStart = $derived(color ?? (isHex(auto) ? auto.toLowerCase() : MARK_PALETTE[0][0]));

  const face = $derived(
    color
      ? `background:${color}`
      : auto
        ? `background:repeating-linear-gradient(135deg, ${auto} 0 3px, var(--bone) 3px 4.5px)`
        : '',
  );

  /// Lives on <body> while open: a table row would otherwise clip it inside
  /// its scroller, and its clicks would bubble up into the row's navigation.
  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    return { destroy: () => node.remove() };
  }

  function place() {
    if (!trigger || !pop) return;
    const t = trigger.getBoundingClientRect();
    const p = pop.getBoundingClientRect();
    const below = t.bottom + 6;
    const top = below + p.height > window.innerHeight - 8 ? t.top - 6 - p.height : below;
    const left = Math.min(t.left, window.innerWidth - p.width - 8);
    pos = { left: Math.max(8, left), top: Math.max(8, top) };
  }

  $effect(() => {
    if (!open || !pop) return;
    place();
    pop.querySelector<HTMLButtonElement>('button')?.focus();

    const close = () => (open = false);
    const onDown = (e: PointerEvent) => {
      const n = e.target as Node;
      if (!pop?.contains(n) && !trigger?.contains(n)) close();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      close();
      trigger?.focus();
    };
    // a fixed popover can't follow its swatch, so any scroll or resize closes it
    window.addEventListener('pointerdown', onDown, true);
    window.addEventListener('keydown', onKey);
    window.addEventListener('scroll', close, true);
    window.addEventListener('resize', close);
    return () => {
      window.removeEventListener('pointerdown', onDown, true);
      window.removeEventListener('keydown', onKey);
      window.removeEventListener('scroll', close, true);
      window.removeEventListener('resize', close);
    };
  });

  async function pick(next: string | null) {
    busy = true;
    try {
      await api.setProjectColor(project, next);
      onchange(next);
      error = '';
      open = false;
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<button
  bind:this={trigger}
  class="pcp-swatch"
  class:auto={!color && auto}
  class:empty={!color && !auto}
  style="width:{size}px;height:{size}px;{face}"
  title={color ? 'Pinned color — click to change' : 'Pin a color to this project'}
  aria-label="Project color"
  aria-haspopup="dialog"
  aria-expanded={open}
  onclick={(e) => {
    e.stopPropagation();
    e.preventDefault();
    open = !open;
  }}
></button>

{#if open}
  <div
    bind:this={pop}
    use:portal
    class="pcp-pop"
    role="dialog"
    aria-label="Pin project color"
    style="left:{pos.left}px;top:{pos.top}px"
  >
    <div class="pcp-hd">Pin color</div>
    <div class="pcp-grid">
      {#each MARK_PALETTE as [hex, name]}
        <button
          class="pcp-sw"
          class:on={color === hex}
          style="background:{hex}"
          title={name}
          aria-label={name}
          aria-pressed={color === hex}
          disabled={busy}
          onclick={() => pick(hex)}
        ></button>
      {/each}
    </div>
    <div class="pcp-row">
      <label class="pcp-custom" title="Any color">
        <input
          type="color"
          value={customStart}
          disabled={busy}
          onchange={(e) => pick(e.currentTarget.value)}
        />
        <span>Custom</span>
      </label>
      {#if color}
        <button class="pcp-clear" disabled={busy} onclick={() => pick(null)}>Clear</button>
      {/if}
    </div>
    {#if error}
      <div class="pcp-err">{error}</div>
    {/if}
  </div>
{/if}

<style>
  .pcp-swatch {
    display: inline-block;
    flex: none;
    padding: 0;
    border: 2px solid var(--ink);
    background: var(--bone);
    vertical-align: middle;
    cursor: pointer;
  }
  .pcp-swatch.empty { border-style: dashed; border-color: var(--dim); }
  .pcp-swatch:hover { outline: 2px solid var(--org); outline-offset: 1px; }
  .pcp-swatch.empty:hover { border-color: var(--ink); }
  .pcp-swatch:focus-visible { outline: 2px solid var(--org); outline-offset: 1px; }

  .pcp-pop {
    position: fixed;
    z-index: 100;
    background: var(--bone);
    border: 2px solid var(--ink);
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .pcp-hd {
    font: 600 10px/1 var(--font-ui);
    letter-spacing: 1.1px;
    text-transform: uppercase;
  }
  .pcp-grid {
    display: grid;
    grid-template-columns: repeat(4, 24px);
    gap: 6px;
  }
  .pcp-sw {
    width: 24px;
    height: 24px;
    padding: 0;
    border: 2px solid var(--ink);
  }
  .pcp-sw:hover,
  .pcp-sw:focus-visible { outline: 2px solid var(--ink); outline-offset: 1px; }
  .pcp-sw.on { outline: 2px solid var(--org); outline-offset: 2px; }

  .pcp-row { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .pcp-custom {
    display: flex;
    align-items: center;
    gap: 6px;
    font: 500 10.5px/1 var(--font-mono);
    letter-spacing: 0.6px;
    text-transform: uppercase;
    cursor: pointer;
  }
  .pcp-custom input {
    width: 24px;
    height: 24px;
    padding: 0;
    border: 2px solid var(--ink);
    border-radius: 0;
    background: var(--bone);
    cursor: pointer;
  }
  .pcp-custom input::-webkit-color-swatch-wrapper { padding: 0; }
  .pcp-custom input::-webkit-color-swatch { border: none; border-radius: 0; }
  .pcp-clear {
    padding: 5px 9px;
    font: 600 10px/1 var(--font-ui);
    letter-spacing: 1.1px;
    text-transform: uppercase;
  }
  .pcp-err {
    max-width: 150px;
    font: 400 10.5px/1.3 var(--font-mono);
    color: var(--mag);
  }
</style>
