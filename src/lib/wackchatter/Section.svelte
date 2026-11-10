<script lang="ts">
  import type { Snippet } from "svelte";
  /**
   * A numbered section that leads with its picture. When it also has a table, a
   * toggle swaps the exact figures in; the choice lives only as long as the page.
   */
  let {
    index,
    kicker,
    title,
    note,
    children,
    table,
    aside,
  }: {
    index: string;
    kicker: string;
    title: string;
    note?: string;
    children: Snippet;
    table?: Snippet;
    aside?: Snippet;
  } = $props();
  let showTable = $state(false);
</script>

<section class="wc-section">
  <header>
    <div>
      <span class="kicker">{index} / {kicker}</span>
      <h2>{title}</h2>
      {#if note}<p class="note">{note}</p>{/if}
    </div>
    <div class="actions">
      {@render aside?.()}
      {#if table}<button
          class="toggle"
          class:on={showTable}
          aria-pressed={showTable}
          onclick={() => (showTable = !showTable)}
          >{showTable ? "Hide table" : "Show table"}</button
        >{/if}
    </div>
  </header>
  {@render children()}
  {#if table && showTable}<div class="table-slot up">{@render table()}</div>{/if}
</section>

<style>
  .wc-section {
    padding: 30px 0 34px;
    border-bottom: 2px solid var(--ink);
  }
  header {
    display: flex;
    align-items: end;
    justify-content: space-between;
    gap: 20px;
    margin-bottom: 20px;
  }
  .kicker {
    font: 11px var(--font-mono);
    letter-spacing: 1.5px;
    text-transform: uppercase;
    color: var(--dim);
  }
  h2 {
    font: 34px/1 var(--font-disp);
    margin: 8px 0 0;
    text-transform: uppercase;
  }
  .note {
    font: 11px/1.6 var(--font-mono);
    color: var(--dim);
    margin: 8px 0 0;
    max-width: 680px;
  }
  .actions {
    display: flex;
    gap: 8px;
    align-items: center;
    flex: none;
  }
  .toggle {
    background: transparent;
    border: 2px solid var(--ink);
    padding: 7px 11px;
    font: 600 10px var(--font-mono);
    letter-spacing: 1px;
    text-transform: uppercase;
    color: var(--ink);
    cursor: pointer;
    border-radius: 0;
  }
  .toggle:hover,
  .toggle.on {
    background: var(--ink);
    color: var(--bone);
  }
  .toggle:focus-visible {
    outline: 3px solid var(--vio);
    outline-offset: 3px;
  }
  .table-slot {
    margin-top: 22px;
  }
</style>
