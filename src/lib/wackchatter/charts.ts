/**
 * ECharts options for the WackChatter destination, built on the shared Marathon
 * fragments. Every builder takes plain folded data and colours; nothing here
 * knows about IPC or the page's state.
 */
import type { EChartsOption } from "echarts";
import { fmtTokens, fmtCost } from "$lib/format";
import { ANIM, AXIS_LABEL, AXIS_LINE, BONE, DIM, HAIR, INK, MONO, SPLIT_LINE, TOOLTIP } from "$lib/chartTheme";
import type { Call, DailyFeature, Flow } from "./types";
import { ESTIMATED, featureColor, featureLabel, OUTCOME_COLORS, count, duration } from "./labels";

const monthTick = (ms: number) =>
  new Date(ms).toLocaleDateString(undefined, { month: "short", year: "2-digit" }).toUpperCase();
const dayTick = (ms: number) =>
  new Date(ms).toLocaleDateString(undefined, { day: "numeric", month: "short" }).toUpperCase();
const esc = (s: string) => s.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);

export interface Lane {
  id: string;
  name: string;
  color: string;
  values: number[];
}

/**
 * Cast eras: one lane per character, each a band whose thickness is that month's
 * share of the lane's own peak, so a quiet character's era still reads as an era.
 */
export function castEras(periods: number[], lanes: Lane[]): EChartsOption {
  const peakOf = new Map(lanes.map((l) => [l.id, Math.max(1, ...l.values)]));
  const series: any[] = lanes.map((lane, li) => {
    const peak = peakOf.get(lane.id)!;
    const y = lanes.length - 1 - li;
    const half = lane.values.map((v) => (v ? Math.max(0.06, (v / peak) * 0.44) : 0));
    return {
      name: lane.name,
      type: "custom",
      // Chart fingerprints an option as JSON, and a renderer's closure is invisible to
      // that. This id carries the colour and shape, so a character whose aura resolves
      // after the first paint re-keys the option and the band repaints.
      id: `${lane.color}|${lane.name}|${half.join(",")}`,
      data: [[0, y]],
      renderItem: (params: any, api: any) => {
        const row = y;
        const pts: number[][] = [];
        const back: number[][] = [];
        half.forEach((h, i) => {
          const x = api.coord([i, row])[0];
          pts.push([x, api.coord([i, row + h])[1]]);
          back.unshift([x, api.coord([i, row - h])[1]]);
        });
        return {
          type: "group",
          children: [
            {
              type: "polygon",
              shape: { points: [...pts, ...back], smooth: 0.35 },
              style: { fill: lane.color, stroke: INK, lineWidth: 1 },
            },
            {
              type: "text",
              silent: true,
              style: {
                x: params.coordSys.x - 12,
                y: api.coord([0, row])[1],
                text: lane.name.length > 16 ? `${lane.name.slice(0, 15).toUpperCase()}…` : lane.name.toUpperCase(),
                align: "right",
                verticalAlign: "middle",
                fill: INK,
                font: `11px ${MONO}`,
              },
            },
          ],
        };
      },
      animationDuration: 900,
      animationDelay: li * 70,
      silent: true,
      z: 2,
    };
  });
  // Invisible hit targets: one per lane × month, so the tooltip reads one month.
  series.push({
    type: "scatter",
    symbolSize: 18,
    itemStyle: { color: "transparent" },
    data: lanes.flatMap((lane, li) =>
      lane.values.map((v, i) => ({
        value: [i, lanes.length - 1 - li],
        name: lane.name,
        period: periods[i],
        replies: v,
      })),
    ),
    z: 5,
  });
  return {
    ...ANIM,
    tooltip: {
      ...TOOLTIP,
      formatter: (p: any) =>
        `${esc(p.data.name)}<br/>${monthTick(p.data.period)} · ${count(p.data.replies)} replies`,
    },
    grid: { left: 124, right: 26, top: 10, bottom: 30 },
    xAxis: {
      type: "value",
      min: 0,
      max: Math.max(1, periods.length - 1),
      interval: Math.max(1, Math.ceil(periods.length / 12)),
      axisLabel: { ...AXIS_LABEL, formatter: (v: number) => (periods[v] === undefined ? "" : monthTick(periods[v]!)) },
      axisLine: { ...AXIS_LINE, onZero: false },
      splitLine: SPLIT_LINE,
    },
    yAxis: {
      type: "value",
      min: -0.5,
      max: lanes.length - 0.5,
      show: false,
    },
    series,
  };
}

const isoDay = (ms: number) => {
  const d = new Date(ms);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
};

/**
 * One row per calendar year, newest at the bottom. The first year starts on its first
 * active day rather than the empty January, so a young library doesn't open with a
 * wall of nothing.
 */
function calendarRows(days: { day: number }[], now = Date.now()): [string, string][] {
  const first = days[0]?.day;
  if (first === undefined) return [];
  const firstYear = new Date(first).getFullYear();
  const lastYear = new Date(now).getFullYear();
  const rows: [string, string][] = [];
  for (let year = firstYear; year <= lastYear; year++) {
    const start = year === firstYear ? isoDay(first) : `${year}-01-01`;
    const end = year === lastYear ? isoDay(now) : `${year}-12-31`;
    rows.push([start, end]);
  }
  return rows;
}

/** Calendar-year rows, Monday-first, bone → lime → ink. */
export function calendarHeat(days: { day: number; count: number }[]): EChartsOption {
  const rows = calendarRows(days);
  const sorted = days.map((d) => d.count).sort((a, b) => a - b);
  const max = Math.max(1, sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * 0.98))] ?? 1);
  return {
    tooltip: {
      ...TOOLTIP,
      formatter: (p: any) => `${dayTick(Date.parse(p.value[0] + "T00:00:00"))}<br/>${count(p.value[1])} messages`,
    },
    visualMap: {
      show: false,
      min: 0,
      max,
      inRange: { color: ["#e0dccd", "#d6e39c", "#b4cf35", "#6f8400", "#3a4400"] },
    },
    calendar: rows.map((range, i) => ({
      range,
      top: 24 + i * 150,
      left: 64,
      right: 8,
      cellSize: ["auto", 15],
      splitLine: { lineStyle: { color: INK, width: 2 } },
      itemStyle: { color: "#ddd8ca", borderColor: BONE, borderWidth: 2 },
      yearLabel: {
        show: true,
        position: "left",
        margin: 30,
        fontFamily: "Anton",
        fontSize: 16,
        color: INK,
        formatter: () => range[1].slice(0, 4),
      },
      monthLabel: { fontFamily: MONO, fontSize: 9.5, color: DIM, nameMap: "EN" },
      dayLabel: { show: true, firstDay: 1, fontFamily: MONO, fontSize: 9, color: DIM, nameMap: ["S", "M", "T", "W", "T", "F", "S"] },
    })),
    series: rows.map((range, i) => ({
      type: "heatmap",
      coordinateSystem: "calendar",
      calendarIndex: i,
      data: days
        .map((d) => [isoDay(d.day), d.count] as [string, number])
        .filter(([day]) => day >= range[0] && day <= range[1]),
    })),
  };
}

export const calendarHeight = (days: { day: number }[]) => calendarRows(days).length * 150 + 20;

const WEEKDAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/**
 * Rhythm clock: 24 hours around the dial, weekdays as rings from Monday at the
 * centre to Sunday at the rim; dot area is the count.
 */
export function rhythmClock(grid: number[][], color: string): EChartsOption {
  const max = Math.max(1, ...grid.flat());
  const data = grid.flatMap((row, d) => row.map((v, h) => [d, h, v]));
  return {
    ...ANIM,
    tooltip: {
      ...TOOLTIP,
      formatter: (p: any) => `${WEEKDAYS[p.value[0]]} ${String(p.value[1]).padStart(2, "0")}:00<br/>${count(p.value[2])} messages`,
    },
    polar: { radius: ["12%", "88%"] },
    angleAxis: {
      type: "category",
      data: Array.from({ length: 24 }, (_, i) => String(i).padStart(2, "0")),
      boundaryGap: false,
      startAngle: 90,
      clockwise: true,
      axisLabel: { ...AXIS_LABEL, interval: 2 },
      axisLine: { lineStyle: { color: INK, width: 2 } },
      splitLine: { show: true, lineStyle: { color: HAIR } },
      axisTick: { show: false },
    },
    radiusAxis: {
      type: "category",
      data: WEEKDAYS,
      axisLabel: { show: false },
      splitLine: { show: true, lineStyle: { color: HAIR } },
      axisLine: { show: false },
      axisTick: { show: false },
    },
    series: [
      {
        type: "scatter",
        coordinateSystem: "polar",
        data,
        symbolSize: (v: number[]) => (v[2] ? 3 + Math.sqrt(v[2] / max) * 17 : 0),
        itemStyle: { color, borderColor: INK, borderWidth: 1 },
        animationDelay: (i: number) => (i % 24) * 18,
      },
    ],
  };
}

/** Streamgraph of swipes per model over local weeks. */
export function modelVoices(periods: number[], lanes: Lane[]): EChartsOption {
  const iso = (ms: number) => {
    const d = new Date(ms);
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  };
  return {
    ...ANIM,
    color: lanes.map((l) => l.color),
    tooltip: {
      ...TOOLTIP,
      trigger: "axis",
      axisPointer: { type: "line", lineStyle: { color: INK, width: 1 } },
    },
    legend: { show: false },
    singleAxis: {
      type: "time",
      top: 10,
      bottom: 30,
      left: 10,
      right: 10,
      splitNumber: 4,
      minInterval: 28 * 86_400_000,
      axisLabel: { ...AXIS_LABEL, hideOverlap: true, formatter: (v: number) => monthTick(v) },
      axisLine: AXIS_LINE,
      splitLine: { show: true, lineStyle: { color: HAIR } },
      axisPointer: { label: { show: false } },
    },
    series: [
      {
        type: "themeRiver",
        label: { show: false },
        itemStyle: { borderColor: INK, borderWidth: 1 },
        emphasis: { itemStyle: { opacity: 1 } },
        data: lanes.flatMap((l) => l.values.map((v, i) => [iso(periods[i]!), v, l.name] as [string, number, string])),
      },
    ],
  };
}

/** Horizontal bars, labelled at the end, coloured per row. */
export function rankBars(
  rows: { name: string; value: number; color: string; note?: string }[],
  format: (n: number) => string,
  opts: { max?: number } = {},
): EChartsOption {
  const ordered = [...rows].reverse();
  return {
    ...ANIM,
    tooltip: {
      ...TOOLTIP,
      formatter: (p: any) => `${esc(p.name)}<br/>${format(p.value)}${ordered[p.dataIndex]?.note ? "<br/>" + ordered[p.dataIndex]!.note : ""}`,
    },
    grid: { left: 8, right: 60, top: 4, bottom: 4, containLabel: true },
    xAxis: { type: "value", max: opts.max, show: false },
    yAxis: {
      type: "category",
      data: ordered.map((r) => r.name),
      axisLabel: { fontFamily: MONO, fontSize: 10.5, color: INK, width: 150, overflow: "truncate" },
      axisLine: AXIS_LINE,
      axisTick: { show: false },
    },
    series: [
      {
        type: "bar",
        barWidth: "62%",
        data: ordered.map((r) => ({ value: r.value, itemStyle: { color: r.color, borderColor: INK, borderWidth: 1 } })),
        label: { show: true, position: "right", fontFamily: MONO, fontSize: 10, color: INK, formatter: (p: any) => format(p.value) },
        animationDelay: (i: number) => (ordered.length - i) * 50,
      },
    ],
  };
}

export const rankBarsHeight = (n: number) => Math.max(80, n * 30 + 10);

/** Personas × cast: square cells, darker is more messages. */
export function personaMatrix(
  personas: string[],
  columns: { name: string; color: string }[],
  cells: [number, number, number][],
): EChartsOption {
  const max = Math.max(1, ...cells.map((c) => c[2]));
  return {
    tooltip: {
      ...TOOLTIP,
      formatter: (p: any) =>
        `${esc(personas[p.value[1]]!)} with ${esc(columns[p.value[0]]!.name)}<br/>${count(p.value[3])} messages`,
    },
    grid: { left: 8, right: 8, top: 8, bottom: 8, containLabel: true },
    xAxis: {
      type: "category",
      data: columns.map((c) => c.name),
      position: "top",
      axisLabel: { fontFamily: MONO, fontSize: 10, color: INK, interval: 0, rotate: columns.length > 6 ? 30 : 0 },
      axisLine: { show: false },
      axisTick: { show: false },
      splitLine: SPLIT_LINE,
    },
    yAxis: {
      type: "category",
      data: personas,
      inverse: true,
      axisLabel: { fontFamily: MONO, fontSize: 10.5, color: INK, width: 170, overflow: "truncate" },
      axisLine: { show: false },
      axisTick: { show: false },
      splitLine: SPLIT_LINE,
    },
    // A cartesian heatmap takes its colour only from a visualMap: one maps the
    // column to that character's aura, a second maps the count to opacity.
    visualMap: [
      {
        show: false,
        type: "piecewise",
        dimension: 0,
        pieces: columns.map((c, i) => ({ value: i, color: c.color })),
      },
      {
        show: false,
        type: "continuous",
        dimension: 2,
        min: 0,
        max: Math.sqrt(max),
        inRange: { colorAlpha: [0.2, 1] },
      },
    ],
    series: [
      {
        type: "heatmap",
        itemStyle: { borderColor: BONE, borderWidth: 3 },
        data: cells.filter((c) => c[2] > 0).map(([x, y, v]) => ({ value: [x, y, Math.sqrt(v), v] })),
        label: { show: true, fontFamily: MONO, fontSize: 10, color: INK, formatter: (p: any) => fmtTokens(p.value[3]) },
        emphasis: { itemStyle: { borderColor: INK, borderWidth: 2 } },
      },
    ],
  };
}

/** A single-value gauge, Marathon style: thick ink track, block progress. */
export function meter(value: number, color: string, label: string): EChartsOption {
  return {
    series: [
      {
        type: "gauge",
        startAngle: 210,
        endAngle: -30,
        min: 0,
        max: 100,
        radius: "92%",
        center: ["50%", "58%"],
        progress: { show: true, width: 18, itemStyle: { color, borderColor: INK, borderWidth: 1 } },
        axisLine: { lineStyle: { width: 18, color: [[1, "#d9d4c6"]] } },
        pointer: { show: false },
        axisTick: { show: false },
        splitLine: { show: false },
        axisLabel: { show: false },
        anchor: { show: false },
        title: { offsetCenter: [0, "38%"], fontFamily: MONO, fontSize: 10, color: DIM },
        detail: { offsetCenter: [0, "0%"], fontFamily: "Anton", fontSize: 30, color: INK, formatter: (v: number) => `${v.toFixed(1)}%` },
        data: [{ value: Math.max(0, Math.min(100, value)), name: label }],
        animationDuration: 1100,
      },
    ],
  };
}

/** Relationship timeline: cumulative replies with milestone flags. */
export function journey(
  days: { day: number; count: number }[],
  marks: { day: number; count: number }[],
  color: string,
): EChartsOption {
  let total = 0;
  const data = days.map((d) => [d.day, (total += d.count)]);
  return {
    ...ANIM,
    tooltip: {
      ...TOOLTIP,
      trigger: "axis",
      formatter: (p: any) => `${dayTick(p[0].value[0])}<br/>${count(p[0].value[1])} replies so far`,
    },
    grid: { left: 52, right: 20, top: 34, bottom: 30 },
    xAxis: { type: "time", axisLabel: { ...AXIS_LABEL, formatter: (v: number) => monthTick(v) }, axisLine: AXIS_LINE, splitLine: { show: false } },
    yAxis: { type: "value", axisLabel: { ...AXIS_LABEL, formatter: (v: number) => fmtTokens(v) }, splitLine: SPLIT_LINE },
    series: [
      {
        type: "line",
        step: "end",
        symbol: "none",
        data,
        lineStyle: { color: INK, width: 2 },
        areaStyle: { color, opacity: 0.85 },
        markPoint: {
          symbol: "pin",
          symbolSize: 34,
          itemStyle: { color: INK },
          label: { fontFamily: MONO, fontSize: 9, color: BONE, formatter: (p: any) => (p.value === 1 ? "1st" : fmtTokens(p.value)) },
          data: marks.map((m) => ({ name: String(m.count), coord: [m.day, data.find((d) => d[0] === m.day)?.[1] ?? m.count], value: m.count })),
        },
      },
    ],
  };
}

/** Speaker share of a group: a donut in each speaker's aura, named in a legend. */
export function share(rows: { name: string; value: number; color: string }[]): EChartsOption {
  return {
    ...ANIM,
    tooltip: { ...TOOLTIP, formatter: (p: any) => `${esc(p.name)}<br/>${count(p.value)} replies (${p.percent}%)` },
    legend: {
      bottom: 0,
      icon: "rect",
      itemWidth: 9,
      itemHeight: 9,
      textStyle: { fontFamily: MONO, fontSize: 10, color: DIM },
    },
    series: [
      {
        type: "pie",
        radius: ["52%", "76%"],
        center: ["50%", "45%"],
        label: { show: false },
        itemStyle: { borderColor: BONE, borderWidth: 3 },
        data: rows.map((r) => ({ name: r.name, value: r.value, itemStyle: { color: r.color } })),
      },
    ],
  };
}

/** Stacked lanes in each speaker's aura over local weeks. */
export function stackedLanes(periods: number[], lanes: Lane[]): EChartsOption {
  return {
    ...ANIM,
    tooltip: { ...TOOLTIP, trigger: "axis" },
    legend: {
      bottom: 0,
      icon: "rect",
      itemWidth: 9,
      itemHeight: 9,
      textStyle: { fontFamily: MONO, fontSize: 10, color: DIM },
    },
    grid: { left: 44, right: 12, top: 12, bottom: 52 },
    xAxis: {
      type: "category",
      data: periods.map(dayTick),
      axisLabel: AXIS_LABEL,
      axisLine: AXIS_LINE,
      boundaryGap: false,
    },
    yAxis: { type: "value", axisLabel: AXIS_LABEL, splitLine: SPLIT_LINE },
    series: lanes.map((l) => ({
      name: l.name,
      type: "line",
      stack: "s",
      symbol: "none",
      smooth: 0.3,
      lineStyle: { width: 1, color: INK },
      areaStyle: { color: l.color, opacity: 0.9 },
      itemStyle: { color: l.color },
      data: l.values,
    })),
  };
}

/** Daily measured tokens stacked by feature, estimates apart, cumulative cost on the right. */
export function generations(rows: DailyFeature[]): EChartsOption {
  const days = [...new Set(rows.map((r) => r.day))].sort();
  const index = new Map(days.map((d, i) => [d, i]));
  const features = [...new Set(rows.map((r) => r.purpose))];
  const totals = new Map(features.map((f) => [f, 0]));
  for (const r of rows) totals.set(r.purpose, totals.get(r.purpose)! + r.measured_tokens);
  features.sort((a, b) => totals.get(b)! - totals.get(a)!);
  const measured = new Map(features.map((f) => [f, new Array<number>(days.length).fill(0)]));
  const estimated = new Array<number>(days.length).fill(0);
  const cost = new Array<number>(days.length).fill(0);
  for (const r of rows) {
    const i = index.get(r.day)!;
    measured.get(r.purpose)![i]! += r.measured_tokens;
    estimated[i]! += r.estimated_tokens;
    cost[i]! += r.cost_usd ?? 0;
  }
  let running = 0;
  const cumulative = cost.map((c) => (running += c));
  const hasEstimates = estimated.some((v) => v > 0);
  return {
    ...ANIM,
    tooltip: {
      ...TOOLTIP,
      trigger: "axis",
      axisPointer: { type: "shadow" },
      valueFormatter: (v: any) => (typeof v === "number" ? fmtTokens(v) : String(v)),
    },
    legend: {
      bottom: 0,
      icon: "rect",
      itemWidth: 10,
      itemHeight: 10,
      textStyle: { fontFamily: MONO, fontSize: 10, color: DIM },
    },
    grid: { left: 52, right: 60, top: 16, bottom: 56 },
    xAxis: {
      type: "category",
      data: days,
      axisLabel: { ...AXIS_LABEL, formatter: (v: string) => dayTick(Date.parse(v + "T00:00:00")) },
      axisLine: AXIS_LINE,
    },
    yAxis: [
      { type: "value", axisLabel: { ...AXIS_LABEL, formatter: (v: number) => fmtTokens(v) }, splitLine: SPLIT_LINE },
      {
        type: "value",
        position: "right",
        axisLabel: {
          ...AXIS_LABEL,
          formatter: (v: number) => `$${v >= 10 ? Math.round(v) : v.toFixed(1)}`,
        },
        splitLine: { show: false },
      },
    ],
    series: [
      ...features.map((f, i) => ({
        name: featureLabel(f),
        type: "bar" as const,
        stack: "measured",
        barCategoryGap: "22%",
        itemStyle: { color: featureColor(f) },
        animationDelay: (idx: number) => idx * 10 + i * 40,
        data: measured.get(f)!,
      })),
      ...(hasEstimates
        ? [
            {
              name: "Estimated (not measured)",
              type: "bar" as const,
              stack: "estimated",
              itemStyle: { color: "transparent", borderColor: ESTIMATED, borderWidth: 1.5, borderType: "dashed" as const },
              data: estimated,
            },
          ]
        : []),
      {
        name: "Cumulative API-equivalent estimate",
        type: "line" as const,
        yAxisIndex: 1,
        symbol: "none",
        step: "end" as const,
        lineStyle: { color: INK, width: 2.5 },
        tooltip: { valueFormatter: (v: any) => fmtCost(Number(v)) },
        data: cumulative,
        z: 10,
      },
    ],
  };
}

/** Feature → model → outcome. Names are prefixed per column so a model can't collide with a feature. */
export function flowSankey(flows: Flow[], weight: "events" | "measured_tokens", modelColor: (m: string) => string): EChartsOption {
  // A library talks to a dozen models; drawn one node each, their names collide. Keep the
  // six that carry the weight and gather the rest, which the table still lists in full.
  const MODEL_NODES = 6;
  const totals = new Map<string, number>();
  // Ranked by calls even when the links are weighted by tokens, so flipping the switch
  // reweights the picture without rearranging which models are on it.
  for (const f of flows) totals.set(f.model, (totals.get(f.model) ?? 0) + f.events);
  const ranked = [...totals].sort((a, b) => b[1] - a[1]).map(([m]) => m);
  const kept = new Set(ranked.slice(0, MODEL_NODES));
  const model = (m: string) => (kept.has(m) ? m : "Other models");

  const nodes = new Map<string, { name: string; label: string; color: string; depth: number }>();
  const links = new Map<string, number>();
  const node = (key: string, label: string, color: string, depth: number) => {
    if (!nodes.has(key)) nodes.set(key, { name: key, label, color, depth });
    return key;
  };
  for (const f of flows) {
    const v = f[weight];
    if (!v) continue;
    const m = model(f.model);
    const a = node(`f:${f.purpose}`, featureLabel(f.purpose), featureColor(f.purpose), 0);
    const b = node(`m:${m}`, m, m === "Other models" ? "#b8b2a4" : modelColor(m), 1);
    const c = node(`o:${f.outcome}`, f.outcome, OUTCOME_COLORS[f.outcome] ?? OUTCOME_COLORS.unknown!, 2);
    links.set(`${a}\u0000${b}`, (links.get(`${a}\u0000${b}`) ?? 0) + v);
    links.set(`${b}\u0000${c}`, (links.get(`${b}\u0000${c}`) ?? 0) + v);
  }
  const fmt = weight === "events" ? (n: number) => `${count(n)} calls` : (n: number) => `${fmtTokens(n)} measured tokens`;
  return {
    ...ANIM,
    tooltip: {
      ...TOOLTIP,
      formatter: (p: any) =>
        p.dataType === "edge"
          ? `${esc(nodes.get(p.data.source)!.label)} → ${esc(nodes.get(p.data.target)!.label)}<br/>${fmt(p.data.value)}`
          : `${esc(nodes.get(p.name)!.label)}<br/>${fmt(p.value)}`,
    },
    series: [
      {
        type: "sankey",
        left: 8,
        right: 118,
        top: 10,
        bottom: 10,
        nodeWidth: 16,
        nodeGap: 14,
        nodeAlign: "justify",
        draggable: false,
        emphasis: { focus: "adjacency" },
        label: {
          fontFamily: MONO,
          fontSize: 10,
          color: INK,
          formatter: (p: any) => nodes.get(p.name)!.label,
        },
        lineStyle: { color: "gradient", opacity: 0.35, curveness: 0.5 },
        itemStyle: { borderColor: INK, borderWidth: 1 },
        data: [...nodes.values()].map((n) => ({ name: n.name, depth: n.depth, itemStyle: { color: n.color } })),
        links: [...links].map(([k, value]) => {
          const [source, target] = k.split("\u0000");
          return { source, target, value };
        }),
      },
    ],
  };
}

/**
 * Beeswarm of one conversation's calls: time across, a deterministic jitter
 * down, area is tokens. Estimated calls are hollow; failures and cancellations
 * change shape so the outcome survives without colour.
 */
export function callSwarm(calls: Call[]): EChartsOption {
  const maxTokens = Math.max(1, ...calls.map((c) => c.measured_tokens + c.estimated_tokens));
  const jitter = (id: string) => {
    let h = 2166136261;
    for (const ch of id) h = Math.imul(h ^ ch.charCodeAt(0), 16777619);
    return ((h >>> 0) % 1000) / 1000 - 0.5;
  };
  const features = [...new Set(calls.map((c) => c.purpose))];
  const shape = (o: string) => (o === "failed" ? "path://M0,0L10,10M10,0L0,10Z" : o === "cancelled" ? "diamond" : "circle");
  return {
    ...ANIM,
    tooltip: {
      ...TOOLTIP,
      formatter: (p: any) => {
        const c: Call = p.data.call;
        const tokens = c.estimated_tokens
          ? `${fmtTokens(c.estimated_tokens)} estimated tokens`
          : `${fmtTokens(c.measured_tokens)} measured tokens`;
        return `${new Date(c.ts).toLocaleString()}<br/>${esc(featureLabel(c.purpose))} · ${esc(c.model)}<br/>${esc(c.outcome)} · ${tokens}<br/>${duration(c.duration_ms)} total · ${duration(c.ttft_ms)} first token${c.cost_usd !== null ? `<br/>${fmtCost(c.cost_usd)} API-equivalent` : ""}`;
      },
    },
    legend: {
      bottom: 0,
      icon: "circle",
      itemWidth: 9,
      itemHeight: 9,
      textStyle: { fontFamily: MONO, fontSize: 10, color: DIM },
    },
    grid: { left: 16, right: 16, top: 12, bottom: 50 },
    xAxis: {
      type: "time",
      splitNumber: 7,
      axisLabel: { ...AXIS_LABEL, hideOverlap: true, formatter: (v: number) => dayTick(v) },
      axisLine: AXIS_LINE,
      splitLine: SPLIT_LINE,
    },
    yAxis: { type: "value", min: -0.6, max: 0.6, show: false },
    series: features.map((f) => ({
      name: featureLabel(f),
      type: "scatter" as const,
      itemStyle: { color: featureColor(f) },
      data: calls
        .filter((c) => c.purpose === f)
        .map((c) => {
          const t = c.measured_tokens + c.estimated_tokens;
          const hollow = c.estimated_tokens > 0;
          return {
            value: [c.ts, jitter(c.id)],
            call: c,
            symbol: shape(c.outcome),
            symbolSize: 5 + Math.sqrt(t / maxTokens) * 20,
            itemStyle: hollow
              ? { color: "transparent", borderColor: ESTIMATED, borderWidth: 1.5 }
              : c.outcome === "failed"
                ? { color: OUTCOME_COLORS.failed, borderColor: OUTCOME_COLORS.failed, borderWidth: 2.5 }
                : { color: featureColor(f), borderColor: INK, borderWidth: 1, opacity: 0.85 },
          };
        }),
    })),
  };
}
