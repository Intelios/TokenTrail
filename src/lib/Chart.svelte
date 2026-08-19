<script lang="ts">
  import * as echarts from 'echarts';
  import { onMount } from 'svelte';

  let {
    option,
    height = 320,
  }: { option?: echarts.EChartsOption; height?: number | 'fill' } = $props();

  let el: HTMLDivElement | undefined = $state();
  let chart = $state<echarts.ECharts | null>(null);

  onMount(() => {
    if (!el) return;
    const c = echarts.init(el);
    const ro = new ResizeObserver(() => c.resize());
    ro.observe(el);
    chart = c;
    requestAnimationFrame(() => c.resize());
    return () => {
      ro.disconnect();
      c.dispose();
      chart = null;
    };
  });

  $effect(() => {
    if (chart && option) {
      chart.setOption(option, true);
      chart.resize();
    }
  });
</script>

<!-- height="fill" stretches to the parent's flexed height instead of a fixed px -->
<div
  bind:this={el}
  style="width:100%;height:{height === 'fill' ? '100%' : height + 'px'};flex:{height === 'fill' ? '1 1 0%' : 'none'};min-height:0;"
></div>
