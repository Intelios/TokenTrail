<script lang="ts">
  import { tick } from 'svelte';
  import { api } from './api';
  import { STACK_PRESETS } from './format';

  let {
    project,
    stack,
    knownTags = [],
    onchange,
  }: {
    project: string;
    stack: string[];
    knownTags?: string[];
    onchange: (next: string[]) => void;
  } = $props();

  let open = $state(false);
  let busy = $state(false);
  let error = $state('');
  let query = $state('');
  let trigger: HTMLButtonElement | undefined = $state();
  let pop: HTMLDivElement | undefined = $state();
  let inputEl: HTMLInputElement | undefined = $state();
  let pos = $state({ left: 0, top: 0 });

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
    tick().then(() => {
      place();
      inputEl?.focus();
    });

    const close = () => {
      open = false;
      query = '';
      error = '';
    };
    const onDown = (e: PointerEvent) => {
      const n = e.target as Node;
      if (!pop?.contains(n) && !trigger?.contains(n)) close();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      close();
      trigger?.focus();
    };
    const onScroll = (e: Event) => {
      if (pop && e.target instanceof Node && pop.contains(e.target)) return;
      close();
    };
    window.addEventListener('pointerdown', onDown, true);
    window.addEventListener('keydown', onKey);
    window.addEventListener('scroll', onScroll, true);
    window.addEventListener('resize', close);
    return () => {
      window.removeEventListener('pointerdown', onDown, true);
      window.removeEventListener('keydown', onKey);
      window.removeEventListener('scroll', onScroll, true);
      window.removeEventListener('resize', close);
    };
  });

  const hasTag = (tag: string) =>
    stack.some((t) => t.toLowerCase() === tag.trim().toLowerCase());

  // User's past tags across all projects first, then built-in presets, deduplicated.
  const allCandidates = $derived.by(() => {
    const seen = new Set<string>();
    const out: { tag: string; custom: boolean }[] = [];
    for (const raw of knownTags) {
      const t = raw.trim();
      const k = t.toLowerCase();
      if (t && !seen.has(k)) {
        seen.add(k);
        out.push({ tag: t, custom: true });
      }
    }
    for (const preset of STACK_PRESETS) {
      const k = preset.toLowerCase();
      if (!seen.has(k)) {
        seen.add(k);
        out.push({ tag: preset, custom: false });
      }
    }
    return out;
  });

  const suggestions = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const available = allCandidates.filter((c) => !hasTag(c.tag));
    if (!q) return available;
    const prefix = available.filter((c) => c.tag.toLowerCase().startsWith(q));
    const rest = available.filter(
      (c) => !c.tag.toLowerCase().startsWith(q) && c.tag.toLowerCase().includes(q),
    );
    return [...prefix, ...rest];
  });

  const trimmedQuery = $derived(query.trim());
  const canAddCustom = $derived(
    trimmedQuery.length > 0 &&
      !hasTag(trimmedQuery) &&
      !suggestions.some((s) => s.tag.toLowerCase() === trimmedQuery.toLowerCase()),
  );

  async function save(next: string[]) {
    busy = true;
    try {
      await api.setProjectStack(project, next);
      onchange(next);
      error = '';
      await tick();
      if (open) place();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function addTag(raw: string) {
    const parts = raw
      .split(',')
      .map((s) => s.trim())
      .filter(Boolean);
    if (!parts.length) return;
    const next = [...stack];
    for (const p of parts) {
      if (!next.some((t) => t.toLowerCase() === p.toLowerCase())) {
        next.push(p);
      }
    }
    query = '';
    if (next.length !== stack.length) {
      await save(next);
    }
    inputEl?.focus();
  }

  async function removeTag(tag: string) {
    const next = stack.filter((t) => t !== tag);
    await save(next);
  }

  function onInputKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' || e.key === ',') {
      e.preventDefault();
      if (trimmedQuery) {
        // If exact match in suggestions, use its canonical casing; otherwise use user's exact input
        const exact = suggestions.find(
          (s) => s.tag.toLowerCase() === trimmedQuery.toLowerCase(),
        );
        addTag(exact ? exact.tag : trimmedQuery);
      }
    } else if (e.key === 'Backspace' && !query && stack.length > 0) {
      removeTag(stack[stack.length - 1]);
    }
  }
</script>

<div class="pse-bar">
  <span class="pse-lbl">Stack:</span>
  {#each stack as tag (tag)}
    <span class="pse-chip">
      <span>{tag}</span>
      <button
        class="pse-x"
        disabled={busy}
        title="Remove {tag}"
        aria-label="Remove {tag}"
        onclick={(e) => {
          e.stopPropagation();
          removeTag(tag);
        }}
      >
        ×
      </button>
    </span>
  {/each}
  <button
    bind:this={trigger}
    class="pse-add"
    class:active={open}
    title="Add or edit project tech stack"
    aria-haspopup="dialog"
    aria-expanded={open}
    onclick={(e) => {
      e.stopPropagation();
      open = !open;
    }}
  >
    {stack.length ? '+ Add' : '+ Tech stack'}
  </button>
</div>

{#if open}
  <div
    bind:this={pop}
    use:portal
    class="pse-pop"
    role="dialog"
    aria-label="Edit project tech stack"
    style="left:{pos.left}px;top:{pos.top}px"
  >
    <div class="pse-hd">
      <span>Tech stack</span>
      {#if stack.length}
        <button class="pse-clear" disabled={busy} onclick={() => save([])}>Clear all</button>
      {/if}
    </div>

    <div class="pse-input-row">
      <input
        bind:this={inputEl}
        type="text"
        placeholder="Type custom stack & press Enter…"
        bind:value={query}
        disabled={busy}
        onkeydown={onInputKeydown}
      />
      {#if trimmedQuery && !hasTag(trimmedQuery)}
        <button
          class="pse-enter-btn"
          disabled={busy}
          onclick={() => addTag(trimmedQuery)}
        >
          Add
        </button>
      {/if}
    </div>

    {#if canAddCustom}
      <button
        class="pse-custom-btn"
        disabled={busy}
        onclick={() => addTag(trimmedQuery)}
      >
        + Add custom <b>"{trimmedQuery}"</b>
      </button>
    {/if}

    <div class="pse-sub">
      {trimmedQuery ? 'Matching suggestions' : 'Suggestions · click to add'}
    </div>
    <div class="pse-sugs">
      {#each suggestions as s (s.tag)}
        <button
          class="pse-sug"
          class:known={s.custom}
          disabled={busy}
          onclick={() => addTag(s.tag)}
          title={s.custom ? 'Used on your projects' : 'Suggested stack'}
        >
          + {s.tag}
        </button>
      {/each}
      {#if !suggestions.length && !canAddCustom}
        <span class="pse-empty">
          {trimmedQuery ? 'Already added to this project' : 'All suggestions added'}
        </span>
      {/if}
    </div>

    {#if error}
      <div class="pse-err">{error}</div>
    {/if}
  </div>
{/if}

<style>
  .pse-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    margin-top: 8px;
  }
  .pse-lbl {
    font: 600 10px/1 var(--font-mono);
    letter-spacing: 0.6px;
    text-transform: uppercase;
    opacity: 0.55;
    margin-right: 2px;
  }
  .pse-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font: 600 10px/1 var(--font-mono);
    letter-spacing: 0.5px;
    text-transform: uppercase;
    padding: 3px 6px;
    border: 1.5px solid var(--ink);
    background: var(--bone);
    color: var(--ink);
  }
  .pse-x {
    background: none;
    border: none;
    padding: 0 1px;
    font: 600 11px/1 var(--font-mono);
    color: var(--dim);
    cursor: pointer;
    text-transform: none;
    letter-spacing: 0;
  }
  .pse-x:hover {
    background: none;
    color: var(--mag);
  }
  .pse-add {
    background: transparent;
    border: 1px dashed var(--ink);
    padding: 3px 8px;
    font: 600 10px/1 var(--font-mono);
    letter-spacing: 0.5px;
    text-transform: uppercase;
    cursor: pointer;
    color: var(--ink);
    opacity: 0.75;
  }
  .pse-add:hover,
  .pse-add.active {
    opacity: 1;
    border-style: solid;
    background: var(--ink);
    color: var(--bone);
  }

  .pse-pop {
    position: fixed;
    z-index: 100;
    width: 310px;
    background: var(--bone);
    border: 2px solid var(--ink);
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .pse-hd {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font: 600 10px/1 var(--font-ui);
    letter-spacing: 1.1px;
    text-transform: uppercase;
  }
  .pse-clear {
    padding: 2px 6px;
    font: 600 9px/1 var(--font-mono);
    letter-spacing: 0.6px;
    border: 1px solid var(--ink);
  }
  .pse-input-row {
    display: flex;
    gap: 6px;
  }
  .pse-input-row input {
    flex: 1;
    padding: 6px 8px;
    font: 500 11.5px/1.2 var(--font-mono);
    border: 2px solid var(--ink);
    background: var(--bone);
    color: var(--ink);
  }
  .pse-input-row input:focus {
    outline: none;
    border-color: var(--org);
  }
  .pse-enter-btn {
    padding: 6px 10px;
    font: 600 10px/1 var(--font-mono);
    background: var(--org);
    color: #fff;
    border: 2px solid var(--ink);
    flex: none;
  }
  .pse-enter-btn:hover {
    background: var(--ink);
    color: var(--bone);
  }
  .pse-custom-btn {
    text-align: left;
    padding: 5px 8px;
    font: 500 10.5px/1.2 var(--font-mono);
    text-transform: none;
    letter-spacing: 0.2px;
    border: 1.5px solid var(--org);
    background: rgba(255, 77, 0, 0.08);
    color: var(--ink);
  }
  .pse-custom-btn:hover {
    background: var(--org);
    color: #fff;
  }
  .pse-sub {
    font: 600 9px/1 var(--font-mono);
    letter-spacing: 0.8px;
    text-transform: uppercase;
    opacity: 0.55;
    margin-top: 2px;
  }
  .pse-sugs {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
    max-height: 156px;
    overflow-y: auto;
    padding-right: 2px;
  }
  .pse-sug {
    padding: 4px 7px;
    font: 500 10px/1 var(--font-mono);
    letter-spacing: 0.4px;
    text-transform: none;
    border: 1px solid rgba(13, 13, 11, 0.3);
    background: rgba(13, 13, 11, 0.03);
    color: var(--ink);
  }
  .pse-sug.known {
    border-color: var(--ink);
    font-weight: 600;
  }
  .pse-sug:hover {
    background: var(--ink);
    color: var(--bone);
    border-color: var(--ink);
  }
  .pse-empty {
    font: 400 10.5px/1.3 var(--font-mono);
    opacity: 0.55;
    padding: 4px 0;
  }
  .pse-err {
    font: 400 10.5px/1.3 var(--font-mono);
    color: var(--mag);
  }
</style>
