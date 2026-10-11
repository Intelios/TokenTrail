<script lang="ts">
  import { wackchatterApi } from "$lib/api";
  let {
    library,
    id,
    name,
    persona = false,
  }: {
    library: string;
    id: string;
    name: string;
    persona?: boolean;
  } = $props();
  let src = $state("");
  $effect(() => {
    let alive = true;
    src = "";
    if (id)
      wackchatterApi
        .portrait(library, id, persona)
        .then((v) => {
          if (alive) src = v;
        })
        .catch(() => {});
    return () => {
      alive = false;
    };
  });
</script>

{#if src}<img {src} alt={name} loading="lazy" />{:else}<span
    class="placeholder"
    aria-label={name}>{name.slice(0, 1).toUpperCase()}</span
  >{/if}

<style>
  img,
  .placeholder {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
    background: var(--acd);
  }
  .placeholder {
    display: grid;
    place-items: center;
    font: 48px var(--font-disp);
    color: var(--ink);
  }
</style>
