/// Daily stacked columns for the overview chart.
///
/// Daily rows arrive from the backend with idle days omitted entirely, so a
/// category axis built straight off them silently deletes time (a month of no
/// usage renders as one step). These helpers lay the rows on a true calendar
/// spine instead, then fold only the *long* idle stretches into a single marked
/// "quiet" band — short gaps stay as real empty slots, because a day or two off
/// is the rhythm of the data rather than dead space worth compressing.
import type { EChartsOption } from 'echarts';
import type { DailyProjectRow, DailyRow, HeatmapCell } from './api';
import { basename, fmtTokens, sourceColor, sourceLabel } from './format';
import {
  TOOLTIP,
  ANIM,
  AXIS_LABEL,
  AXIS_LINE,
  SPLIT_LINE,
  DIM,
  MONO,
  stackedColumn,
  type ChartColor,
} from './chartTheme';

const DAY = 86_400_000;

/// Idle runs shorter than this stay as blank slots — a long weekend reads as
/// rhythm, a fortnight off reads as a hole worth labelling.
const MIN_QUIET_RUN = 3;

export type Column =
  | { kind: 'day'; start: number; values: number[]; total: number }
  | { kind: 'quiet'; start: number; days: number };

const utc = (date: string) => Date.parse(`${date}T00:00:00Z`);
const iso = (t: number) => new Date(t).toISOString().slice(0, 10);

/** "AUG 10" — uppercase mono tick, UTC so it matches the bucket it labels. */
function tick(t: number): string {
  const d = new Date(t);
  const mon = d.toLocaleDateString('en-US', { month: 'short', timeZone: 'UTC' }).toUpperCase();
  return `${mon} ${d.getUTCDate()}`;
}

/** "SAT, AUG 15" — the tooltip title; the weekday explains a lot of quiet days. */
function longTick(t: number): string {
  const wd = new Date(t).toLocaleDateString('en-US', { weekday: 'short', timeZone: 'UTC' }).toUpperCase();
  return `${wd}, ${tick(t)}`;
}

const span = (from: number, to: number) => `${tick(from)} – ${tick(to)}`;

const quietLabel = (days: number) => `${days} QUIET ${days === 1 ? 'DAY' : 'DAYS'}`;

/**
 * Lay `daily` on a gapless calendar spine from the first active day to the last,
 * folding every idle run of `MIN_QUIET_RUN` days or more into one `quiet`
 * column. Leading and trailing dormancy falls outside the spine.
 */
export function dailyColumns(daily: DailyRow[], sources: string[]): Column[] {
  const buckets = new Map<number, number[]>();
  let first = Infinity;
  let last = -Infinity;

  for (const r of daily) {
    if (r.tokens <= 0) continue;
    const i = sources.indexOf(r.source);
    if (i < 0) continue;
    const t = utc(r.date);
    const values = buckets.get(t) ?? sources.map(() => 0);
    values[i] += r.tokens;
    buckets.set(t, values);
    if (t < first) first = t;
    if (t > last) last = t;
  }
  if (!buckets.size) return [];

  const cols: Column[] = [];
  let idle = 0;

  // an idle run is only worth a band once it is long enough to notice
  const flush = (until: number) => {
    if (!idle) return;
    const start = until - idle * DAY;
    if (idle >= MIN_QUIET_RUN) {
      cols.push({ kind: 'quiet', start, days: idle });
    } else {
      for (let k = 0; k < idle; k++) {
        cols.push({ kind: 'day', start: start + k * DAY, values: sources.map(() => 0), total: 0 });
      }
    }
    idle = 0;
  };

  for (let t = first; t <= last; t += DAY) {
    const values = buckets.get(t);
    if (!values) {
      idle++;
      continue;
    }
    flush(t);
    cols.push({ kind: 'day', start: t, values, total: values.reduce((a, b) => a + b, 0) });
  }
  return cols;
}

/** Day totals across the spine, idle days counted as the zeros they are. */
export function spineTotals(cols: Column[]): number[] {
  return cols.flatMap((c) => (c.kind === 'day' ? [c.total] : Array<number>(c.days).fill(0)));
}

/** "APR 23 – AUG 21" — the span the chart actually covers, for the header. */
export function dailyRange(cols: Column[]): string {
  if (!cols.length) return '';
  const last = cols[cols.length - 1];
  return span(cols[0].start, last.start + (last.kind === 'quiet' ? (last.days - 1) * DAY : 0));
}

export function dailyOption(cols: Column[], sources: string[]): EChartsOption | undefined {
  if (!cols.length) return undefined;

  const cats = cols.map((c, i) => (c.kind === 'day' ? iso(c.start) : `quiet-${i}`));
  // keyed by category value — an index-based formatter breaks the moment the
  // axis window shifts (dataZoom hands the formatter the *visible* index)
  const labels = new Map(cols.map((c, i) => [cats[i], c.kind === 'day' ? tick(c.start) : '']));

  // Quiet bands ride a hidden 0..1 axis so they always stand full height without
  // having to know the token scale. They were markAreas first, but ECharts parks
  // the labels of two nearby markAreas at the same x and one vanishes under the
  // other; a real bar series is positioned per category, so it can't collide.
  const quietText = cols.map((c) => (c.kind === 'quiet' ? quietLabel(c.days) : ''));
  const hasQuiet = cols.some((c) => c.kind === 'quiet');

  return {
    backgroundColor: 'transparent',
    ...ANIM,
    tooltip: {
      trigger: 'axis',
      ...TOOLTIP,
      confine: true,
      axisPointer: { type: 'shadow', shadowStyle: { color: 'rgba(13,13,11,0.06)' } },
      formatter: (params: unknown) => {
        type TipParam = { dataIndex: number; marker: string; seriesName: string; value?: number };
        const all = params as TipParam[];
        const col = cols[all[0]?.dataIndex ?? -1];
        if (!col) return '';
        if (col.kind === 'quiet') {
          return (
            `<b>${col.days} quiet ${col.days === 1 ? 'day' : 'days'}</b><br/>` +
            `<span style="opacity:0.65;">${span(col.start, col.start + (col.days - 1) * DAY)}</span><br/>no usage`
          );
        }
        const title = `<b>${longTick(col.start)}</b>`;
        const rows = all.filter((p) => Number(p.value ?? 0) > 0);
        if (!rows.length) return `${title}<br/>no usage`;
        return (
          title +
          '<br/>' +
          rows
            .map((p) => `${p.marker}${p.seriesName}&nbsp;&nbsp;<b>${fmtTokens(Number(p.value))}</b>`)
            .join('<br/>') +
          `<div style="border-top:1px solid rgba(232,228,217,0.3);margin-top:6px;padding-top:5px;display:flex;justify-content:space-between;gap:24px;">` +
          `<span style="opacity:0.65;letter-spacing:1px;">TOTAL</span><b>${fmtTokens(col.total)}</b></div>`
        );
      },
    },
    grid: { left: 4, right: 8, top: 12, bottom: 0, containLabel: true },
    xAxis: {
      type: 'category',
      data: cats,
      axisLine: AXIS_LINE,
      axisTick: { show: false },
      axisLabel: { ...AXIS_LABEL, hideOverlap: true, formatter: (v: string) => labels.get(v) ?? '' },
    },
    yAxis: [
      {
        type: 'value',
        min: 0,
        axisLabel: { ...AXIS_LABEL, formatter: (v: number) => fmtTokens(v) },
        splitLine: SPLIT_LINE,
      },
      // geometry only — carries the full-height quiet bands, never shown
      { type: 'value', min: 0, max: 1, show: false },
    ],
    series: [
      ...sources.map((s, i) =>
        stackedColumn(
          sourceLabel(s),
          cols.map((c) => (c.kind === 'day' ? c.values[i] || null : null)),
          sourceColor(s),
          { delay: i * 60 },
        ),
      ),
      ...(hasQuiet
        ? [
            {
              name: 'quiet',
              type: 'bar' as const,
              yAxisIndex: 1,
              // sit on top of the data group rather than beside it, and behind it
              barGap: '-100%',
              z: 0,
              silent: true,
              itemStyle: { color: 'rgba(13,13,11,0.05)' },
              label: {
                show: true,
                position: 'inside' as const,
                rotate: 90,
                color: DIM,
                fontFamily: MONO,
                fontSize: 9,
                formatter: (p: { dataIndex: number }) => quietText[p.dataIndex],
              },
              data: cols.map((c) => (c.kind === 'quiet' ? 1 : null)),
            },
          ]
        : []),
    ],
  } satisfies EChartsOption;
}

export type SingleDailyColumn =
  | { kind: 'day'; start: number; date: string; tokens: number }
  | { kind: 'quiet'; start: number; days: number };

/**
 * Lay single-series daily usage (HeatmapCell[]) on a gapless calendar spine
 * from the first active day to the last, folding every idle run of MIN_QUIET_RUN
 * days or more into one quiet column.
 */
export function singleDailyColumns(daily: HeatmapCell[]): SingleDailyColumn[] {
  const buckets = new Map<number, number>();
  let first = Infinity;
  let last = -Infinity;

  for (const r of daily) {
    if (r.tokens <= 0) continue;
    const t = utc(r.date);
    buckets.set(t, (buckets.get(t) ?? 0) + r.tokens);
    if (t < first) first = t;
    if (t > last) last = t;
  }
  if (!buckets.size) return [];

  const cols: SingleDailyColumn[] = [];
  let idle = 0;

  const flush = (until: number) => {
    if (!idle) return;
    const start = until - idle * DAY;
    if (idle >= MIN_QUIET_RUN) {
      cols.push({ kind: 'quiet', start, days: idle });
    } else {
      for (let k = 0; k < idle; k++) {
        const d = start + k * DAY;
        cols.push({ kind: 'day', start: d, date: iso(d), tokens: 0 });
      }
    }
    idle = 0;
  };

  for (let t = first; t <= last; t += DAY) {
    const tokens = buckets.get(t);
    if (tokens === undefined) {
      idle++;
      continue;
    }
    flush(t);
    cols.push({ kind: 'day', start: t, date: iso(t), tokens });
  }
  return cols;
}

export function singleDailyOption(
  cols: SingleDailyColumn[],
  color: ChartColor,
): EChartsOption | undefined {
  if (!cols.length) return undefined;

  const cats = cols.map((c, i) => (c.kind === 'day' ? c.date : `quiet-${i}`));
  const labels = new Map(cols.map((c, i) => [cats[i], c.kind === 'day' ? tick(c.start) : '']));

  const quietText = cols.map((c) => (c.kind === 'quiet' ? quietLabel(c.days) : ''));
  const hasQuiet = cols.some((c) => c.kind === 'quiet');

  return {
    backgroundColor: 'transparent',
    ...ANIM,
    animationDelay: (idx: number) => idx * 8,
    tooltip: {
      trigger: 'axis',
      ...TOOLTIP,
      confine: true,
      axisPointer: { type: 'shadow', shadowStyle: { color: 'rgba(13,13,11,0.06)' } },
      formatter: (params: unknown) => {
        type TipParam = { dataIndex: number; marker: string; seriesIndex: number; value?: number };
        const all = params as TipParam[];
        const col = cols[all[0]?.dataIndex ?? -1];
        if (!col) return '';
        if (col.kind === 'quiet') {
          return (
            `<b>${col.days} quiet ${col.days === 1 ? 'day' : 'days'}</b><br/>` +
            `<span style="opacity:0.65;">${span(col.start, col.start + (col.days - 1) * DAY)}</span><br/>no usage`
          );
        }
        if (col.tokens > 0) {
          const marker = all.find((p) => p.seriesIndex === 0)?.marker ?? all[0]?.marker ?? '';
          return (
            `<div style="margin:0;line-height:1;">` +
            `<div style="font-size:11px;color:#e8e4d9;font-weight:400;line-height:1;">${col.date}</div>` +
            `<div style="margin:10px 0 0;line-height:1;">` +
            `<div style="margin:0;line-height:1;">` +
            `${marker}<span style="float:right;margin-left:10px;font-size:11px;color:#e8e4d9;font-weight:900;">${fmtTokens(col.tokens)}</span>` +
            `<div style="clear:both"></div>` +
            `</div><div style="clear:both"></div>` +
            `</div><div style="clear:both"></div>` +
            `</div>`
          );
        }
        return (
          `<div style="margin:0;line-height:1;">` +
          `<div style="font-size:11px;color:#e8e4d9;font-weight:400;line-height:1;">${col.date}</div>` +
          `<div style="margin:10px 0 0;line-height:1;color:rgba(232,228,217,0.65);">no usage</div>` +
          `</div>`
        );
      },
    },
    grid: { left: 8, right: 8, top: 20, bottom: 0, containLabel: true },
    xAxis: {
      type: 'category',
      data: cats,
      axisLine: AXIS_LINE,
      axisTick: { show: false },
      axisLabel: { ...AXIS_LABEL, hideOverlap: true, formatter: (v: string) => labels.get(v) ?? '' },
    },
    yAxis: [
      {
        type: 'value',
        min: 0,
        axisLabel: { ...AXIS_LABEL, formatter: (v: number) => fmtTokens(v) },
        splitLine: SPLIT_LINE,
      },
      { type: 'value', min: 0, max: 1, show: false },
    ],
    series: [
      {
        type: 'bar' as const,
        barMaxWidth: 40,
        data: cols.map((c) => (c.kind === 'day' ? (c.tokens > 0 ? c.tokens : null) : null)),
        itemStyle: { color },
        animationDelay: (idx: number) => idx * 8,
      },
      ...(hasQuiet
        ? [
            {
              name: 'quiet',
              type: 'bar' as const,
              yAxisIndex: 1,
              barGap: '-100%',
              z: 0,
              silent: true,
              itemStyle: { color: 'rgba(13,13,11,0.05)' },
              label: {
                show: true,
                position: 'inside' as const,
                rotate: 90,
                color: DIM,
                fontFamily: MONO,
                fontSize: 9,
                formatter: (p: { dataIndex: number }) => quietText[p.dataIndex],
              },
              data: cols.map((c) => (c.kind === 'quiet' ? 1 : null)),
            },
          ]
        : []),
    ],
  } satisfies EChartsOption;
}

export function disambiguateProjectNames(paths: string[]): string[] {
  const counts = new Map<string, number>();
  for (const p of paths) {
    const b = basename(p);
    counts.set(b, (counts.get(b) ?? 0) + 1);
  }
  return paths.map((p) => {
    const b = basename(p);
    if ((counts.get(b) ?? 0) > 1) {
      const parts = p.split(/[/\\]/).filter(Boolean);
      if (parts.length >= 2) {
        return `${parts[parts.length - 2]}/${parts[parts.length - 1]}`;
      }
    }
    return b;
  });
}

/**
 * Lay project daily usage on a gapless calendar spine from the first active day to the last,
 * folding idle stretches into a quiet column.
 */
export function projectDailyColumns(daily: DailyProjectRow[], projects: string[]): Column[] {
  const buckets = new Map<number, number[]>();
  let first = Infinity;
  let last = -Infinity;

  for (const r of daily) {
    if (r.tokens <= 0) continue;
    const i = projects.indexOf(r.project);
    if (i < 0) continue;
    const t = utc(r.date);
    const values = buckets.get(t) ?? projects.map(() => 0);
    values[i] += r.tokens;
    buckets.set(t, values);
    if (t < first) first = t;
    if (t > last) last = t;
  }
  if (!buckets.size) return [];

  const cols: Column[] = [];
  let idle = 0;

  const flush = (until: number) => {
    if (!idle) return;
    const start = until - idle * DAY;
    if (idle >= MIN_QUIET_RUN) {
      cols.push({ kind: 'quiet', start, days: idle });
    } else {
      for (let k = 0; k < idle; k++) {
        cols.push({ kind: 'day', start: start + k * DAY, values: projects.map(() => 0), total: 0 });
      }
    }
    idle = 0;
  };

  for (let t = first; t <= last; t += DAY) {
    const values = buckets.get(t);
    if (!values) {
      idle++;
      continue;
    }
    flush(t);
    cols.push({ kind: 'day', start: t, values, total: values.reduce((a, b) => a + b, 0) });
  }
  return cols;
}

/** `colors[i]` paints `projects[i]` — callers resolve pinned, auto and muted colors. */
export function projectDailyOption(
  cols: Column[],
  projects: string[],
  colors: ChartColor[],
): EChartsOption | undefined {
  if (!cols.length) return undefined;

  const cats = cols.map((c, i) => (c.kind === 'day' ? iso(c.start) : `quiet-${i}`));
  const labels = new Map(cols.map((c, i) => [cats[i], c.kind === 'day' ? tick(c.start) : '']));
  const quietText = cols.map((c) => (c.kind === 'quiet' ? quietLabel(c.days) : ''));
  const hasQuiet = cols.some((c) => c.kind === 'quiet');
  const displayNames = disambiguateProjectNames(projects);

  return {
    backgroundColor: 'transparent',
    ...ANIM,
    tooltip: {
      trigger: 'axis',
      ...TOOLTIP,
      confine: true,
      axisPointer: { type: 'shadow', shadowStyle: { color: 'rgba(13,13,11,0.06)' } },
      formatter: (params: unknown) => {
        type TipParam = { dataIndex: number; marker: string; seriesName: string; value?: number };
        const all = params as TipParam[];
        const col = cols[all[0]?.dataIndex ?? -1];
        if (!col) return '';
        if (col.kind === 'quiet') {
          return (
            `<b>${col.days} quiet ${col.days === 1 ? 'day' : 'days'}</b><br/>` +
            `<span style="opacity:0.65;">${span(col.start, col.start + (col.days - 1) * DAY)}</span><br/>no usage`
          );
        }
        const title = `<b>${longTick(col.start)}</b>`;
        const rows = all.filter((p) => Number(p.value ?? 0) > 0);
        if (!rows.length) return `${title}<br/>no usage`;
        return (
          title +
          '<br/>' +
          rows
            .map((p) => `${p.marker}${p.seriesName}&nbsp;&nbsp;<b>${fmtTokens(Number(p.value))}</b>`)
            .join('<br/>') +
          `<div style="border-top:1px solid rgba(232,228,217,0.3);margin-top:6px;padding-top:5px;display:flex;justify-content:space-between;gap:24px;">` +
          `<span style="opacity:0.65;letter-spacing:1px;">TOTAL</span><b>${fmtTokens(col.total)}</b></div>`
        );
      },
    },
    grid: { left: 4, right: 8, top: 12, bottom: 0, containLabel: true },
    xAxis: {
      type: 'category',
      data: cats,
      axisLine: AXIS_LINE,
      axisTick: { show: false },
      axisLabel: { ...AXIS_LABEL, hideOverlap: true, formatter: (v: string) => labels.get(v) ?? '' },
    },
    yAxis: [
      {
        type: 'value',
        min: 0,
        axisLabel: { ...AXIS_LABEL, formatter: (v: number) => fmtTokens(v) },
        splitLine: SPLIT_LINE,
      },
      { type: 'value', min: 0, max: 1, show: false },
    ],
    series: [
      ...projects.map((_p, i) =>
        stackedColumn(
          displayNames[i],
          cols.map((c) => (c.kind === 'day' ? c.values[i] || null : null)),
          colors[i],
          { delay: i * 40 },
        ),
      ),
      ...(hasQuiet
        ? [
            {
              name: 'quiet',
              type: 'bar' as const,
              yAxisIndex: 1,
              barGap: '-100%',
              z: 0,
              silent: true,
              itemStyle: { color: 'rgba(13,13,11,0.05)' },
              label: {
                show: true,
                position: 'inside' as const,
                rotate: 90,
                color: DIM,
                fontFamily: MONO,
                fontSize: 9,
                formatter: (p: { dataIndex: number }) => quietText[p.dataIndex],
              },
              data: cols.map((c) => (c.kind === 'quiet' ? 1 : null)),
            },
          ]
        : []),
    ],
  } satisfies EChartsOption;
}


