<script lang="ts">
  import { onMount } from 'svelte';
  import { sparkPathD } from '$lib/spark';

  /**
   * Inline SVG sparkline that draws itself in (stroke-dashoffset animation),
   * mirroring the .trc treatment in design/design-marathon.html.
   */
  let {
    values,
    width = 94,
    height = 20,
    color = 'var(--org)',
    stroke = 1.4,
    delay = 0,
  }: {
    values: number[];
    width?: number;
    height?: number;
    color?: string;
    stroke?: number;
    delay?: number;
  } = $props();

  let pathEl: SVGPathElement | undefined = $state();
  const d = $derived(sparkPathD(values, width, height));

  onMount(() => {
    if (!pathEl) return;
    if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;
    const len = Math.ceil(pathEl.getTotalLength()) + 2;
    pathEl.style.strokeDasharray = String(len);
    pathEl.style.strokeDashoffset = String(len);
    // two frames so the initial dashoffset is committed before transitioning
    requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        pathEl!.style.transition = `stroke-dashoffset 1.1s cubic-bezier(.3,0,.15,1) ${delay}ms`;
        pathEl!.style.strokeDashoffset = '0';
      }),
    );
  });
</script>

<svg width={width} height={height} viewBox="0 0 {width} {height}" preserveAspectRatio="none" style="display:block">
  <path bind:this={pathEl} {d} fill="none" stroke={color} stroke-width={stroke} />
</svg>
