<script lang="ts" module>
  const reduced = () => window.matchMedia('(prefers-reduced-motion: reduce)').matches;
</script>

<script lang="ts">
  import { untrack } from "svelte";
  /**
   * Count-up number: animates from the previously shown value to the target
   * with an ease-out curve. Full sweep on mount, short re-sweep on data
   * refreshes (tt-sync), instant when reduced motion is preferred.
   */
  let {
    value,
    format = (n: number) => Math.round(n).toLocaleString(),
    duration = 900,
  }: {
    value: number;
    format?: (n: number) => string;
    duration?: number;
  } = $props();

  let display = $state(0);
  let raf = 0;

  function animateTo(target: number, ms: number) {
    cancelAnimationFrame(raf);
    if (reduced() || ms <= 0) {
      display = target;
      return;
    }
    const origin = display;
    const start = performance.now();
    const tick = (now: number) => {
      // a frame stamp can land before the start instant under load or virtual time,
      // and a negative t would drive the ease past both ends
      const t = Math.max(0, Math.min(1, (now - start) / ms));
      const e = 1 - Math.pow(1 - t, 3);
      display = origin + (target - origin) * e;
      if (t < 1) raf = requestAnimationFrame(tick);
      else display = target;
    };
    raf = requestAnimationFrame(tick);
  }

  // display is written by the animation this effect starts, so reading it untracked
  // keeps the loop out of the effect's own dependencies
  $effect(() => {
    const target = value;
    const first = untrack(() => display) === 0 && target !== 0;
    animateTo(target, first ? duration : 300);
  });

  $effect(() => () => cancelAnimationFrame(raf));
</script>

<span>{format(display)}</span>

<style>
  span {
    font-variant-numeric: tabular-nums;
  }
</style>
