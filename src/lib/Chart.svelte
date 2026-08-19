<script lang="ts">
  import * as echarts from 'echarts';
  import { onMount } from 'svelte';

  let {
    option,
    height = 320,
  }: { option?: echarts.EChartsOption; height?: number } = $props();

  let el: HTMLDivElement | undefined = $state();
  let chart = $state<echarts.ECharts | null>(null);

  onMount(() => {
    if (!el) return;
    const c = echarts.init(el);
    const ro = new ResizeObserver(() => c.resize());
    ro.observe(el);
    chart = c;
    return () => {
      ro.disconnect();
      c.dispose();
      chart = null;
    };
  });

  $effect(() => {
    if (chart && option) chart.setOption(option, true);
  });
</script>

<div bind:this={el} style="width:100%;height:{height}px"></div>
