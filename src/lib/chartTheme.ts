/// Marathon chart theme — shared ECharts option fragments.
/// Every page builds its option on top of these so tooltips, axes, legends
/// and animation choreography stay consistent across the app.

export const INK = '#0d0d0b';
export const BONE = '#e8e4d9';
export const HAIR = 'rgba(13,13,11,0.16)';
export const DIM = 'rgba(13,13,11,0.55)';
export const MONO = "'IBM Plex Mono', monospace";

/** Ink tooltip with bone text — hard corners, no shadow. */
export const TOOLTIP = {
  backgroundColor: INK,
  borderColor: INK,
  borderWidth: 2,
  borderRadius: 0,
  padding: [8, 12] as [number, number],
  textStyle: { color: BONE, fontFamily: MONO, fontSize: 11 },
  extraCssText: 'box-shadow:none;',
};

export const AXIS_LABEL = { color: DIM, fontFamily: MONO, fontSize: 9.5 };
export const AXIS_LINE = { lineStyle: { color: INK, width: 2 } };
export const SPLIT_LINE = { lineStyle: { color: HAIR, width: 1 } };

export const LEGEND_TEXT = { color: DIM, fontFamily: MONO, fontSize: 10 };

/** Compact uppercase date tick for category axes ("2026-04-22" → "APR 22"). */
export function dateTick(v: string): string {
  const d = new Date(v + 'T00:00:00');
  if (Number.isNaN(d.getTime())) return v;
  return `${d.toLocaleDateString('en-US', { month: 'short' }).toUpperCase()} ${d.getDate()}`;
}

/** Base animation block — spread at the option root. */
export const ANIM = {
  animationDuration: 700,
  animationEasing: 'cubicOut' as const,
};

/** Marathon stacked band: hard edges, area fill without overlapping strokes on 0-value series. */
export function stackedBand(
  name: string,
  data: Array<number | null>,
  color: string,
  opts: { delay?: number; stack?: string; lineWidth?: number; opacity?: number } = {},
) {
  return {
    name,
    type: 'line' as const,
    stack: opts.stack ?? 'total',
    smooth: false,
    symbol: 'none',
    lineStyle: { width: opts.lineWidth ?? 0, color },
    itemStyle: { color },
    areaStyle: { opacity: opts.opacity ?? 0.75 },
    emphasis: { focus: 'series' as const },
    animationDelay: opts.delay ?? 0,
    data,
  };
}

/** Donut with 2px bone slice borders, slices sweeping in staggered. */
export function donutSeries(
  data: Array<{ name: string; value: number; itemStyle?: Record<string, unknown> }>,
) {
  return {
    type: 'pie' as const,
    radius: ['58%', '82%'] as [string, string],
    label: { show: false },
    itemStyle: { borderColor: BONE, borderWidth: 2 },
    animationType: 'scale' as const,
    animationDuration: 700,
    animationEasing: 'cubicOut' as const,
    animationDelay: (idx: number) => idx * 60,
    data,
  };
}
