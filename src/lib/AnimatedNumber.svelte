<script lang="ts" module>
  const reduced = () => window.matchMedia('(prefers-reduced-motion: reduce)').matches;
</script>

<script lang="ts">
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
      const t = Math.min(1, (now - start) / ms);
      const e = 1 - Math.pow(1 - t, 3);
      display = origin + (target - origin) * e;
      if (t < 1) raf = requestAnimationFrame(tick);
      else display = target;
    };
    raf = requestAnimationFrame(tick);
  }

  $effect(() => {
    const first = display === 0 && value !== 0;
    animateTo(value, first ? duration : 300);
  });

  $effect(() => () => cancelAnimationFrame(raf));
</script>

<span>{format(display)}</span>

<style>
  span {
    font-variant-numeric: tabular-nums;
  }
</style>
