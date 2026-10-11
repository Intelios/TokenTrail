<script lang="ts">
  import * as echarts from 'echarts';
  import { onMount } from 'svelte';

  let {
    option,
    height = 320,
    onclick,
  }: {
    option?: echarts.EChartsOption;
    height?: number | 'fill';
    onclick?: (params: any) => void;
  } = $props();

  let reducedMotion = $state(false);
  let el: HTMLDivElement | undefined = $state();
  let chart = $state<echarts.ECharts | null>(null);
  // Content key of the option last applied: parent pages rebuild their option
  // object on every tt-sync refresh even when the numbers are identical, and
  // applying it again would replay the entrance animation each tick.
  let lastKey = '';
  const optionKey = (o: echarts.EChartsOption) => {
    try {
      return JSON.stringify(o, (_, v) => (typeof v === 'function' ? v.toString() : v));
    } catch {
      return null;
    }
  };

  onMount(() => {
    if (!el) return;
    const media = window.matchMedia('(prefers-reduced-motion: reduce)');
    const motionChanged = () => { reducedMotion = media.matches; };
    motionChanged();
    media.addEventListener('change', motionChanged);
    const c = echarts.init(el);
    const ro = new ResizeObserver(() => c.resize());
    ro.observe(el);
    chart = c;
    requestAnimationFrame(() => c.resize());
    return () => {
      media.removeEventListener('change', motionChanged);
      ro.disconnect();
      c.dispose();
      chart = null;
      lastKey = '';
    };
  });

  $effect(() => {
    if (chart && option) {
      const content = optionKey(option);
      // content===null means the option can't be fingerprinted — apply it
      // unconditionally rather than pinning lastKey to a constant
      if (content !== null && JSON.stringify([content, reducedMotion]) === lastKey)
        return;
      lastKey = content === null ? '' : JSON.stringify([content, reducedMotion]);
      chart.setOption({ ...option, animation: reducedMotion ? false : option.animation }, true);
      chart.resize();
    }
  });

  $effect(() => {
    if (chart) {
      chart.off('click');
      if (onclick) {
        chart.on('click', (params) => onclick(params));
      }
    }
  });
</script>

<!-- height="fill" stretches to the parent's flexed height instead of a fixed px -->
<div
  bind:this={el}
  style="width:100%;height:{height === 'fill' ? '100%' : height + 'px'};flex:{height === 'fill' ? '1 1 0%' : 'none'};min-height:0;"
></div>
