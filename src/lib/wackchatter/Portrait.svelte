<script lang="ts">
  import { portraitSrc } from "./portraits";
  let {
    library,
    id,
    name,
    persona = false,
    tint = "var(--acd)",
    letter = 48,
  }: {
    library: string;
    id: string;
    name: string;
    persona?: boolean;
    tint?: string;
    letter?: number;
  } = $props();
  let src = $state("");
  $effect(() => {
    let alive = true;
    src = "";
    if (id)
      void portraitSrc(library, id, persona).then((v) => {
        if (alive) src = v;
      });
    return () => {
      alive = false;
    };
  });
</script>

{#if src}<img {src} alt={name} loading="lazy" />{:else}<span
    class="placeholder"
    style="background:{tint};font-size:{letter}px"
    aria-label={name}>{name.slice(0, 1).toUpperCase()}</span
  >{/if}

<style>
  img,
  .placeholder {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .placeholder {
    display: grid;
    place-items: center;
    font-family: var(--font-disp);
    color: var(--ink);
  }
</style>
