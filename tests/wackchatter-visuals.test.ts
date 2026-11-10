import { describe, expect, test } from "bun:test";
import {
  auraFrom,
  contrastRatio,
  FALLBACK,
  hexToOklch,
  resolveAura,
  selectPalette,
} from "../src/lib/wackchatter/aura";
import {
  foldActivity,
  foldDays,
  foldWeekHour,
  milestones,
  startOfPeriod,
} from "../src/lib/wackchatter/buckets";

const BONE = "#e8e4d9";

function withZone<T>(zone: string, run: () => T): T {
  const previous = process.env.TZ;
  process.env.TZ = zone;
  try {
    return run();
  } finally {
    if (previous === undefined) delete process.env.TZ;
    else process.env.TZ = previous;
  }
}

describe("week × hour", () => {
  test("files instants under local weekday and hour", () => {
    withZone("Asia/Kathmandu", () => {
      // 18:15Z on Sunday is 00:00 Monday in +5:45
      const grid = foldWeekHour([[Date.parse("2026-08-09T18:15:00Z"), 3]]);
      expect(grid[0]![0]).toBe(3);
      expect(grid.flat().reduce((a, b) => a + b, 0)).toBe(3);
    });
  });
});

describe("activity periods", () => {
  test("months are contiguous and per-id totals survive", () => {
    withZone("Europe/London", () => {
      const rows: [string, number, number][] = [
        ["a", Date.parse("2026-01-15T12:00:00Z"), 2],
        ["b", Date.parse("2026-03-31T23:30:00Z"), 5],
        ["a", Date.parse("2026-03-29T00:30:00Z"), 1],
      ];
      const { periods, series } = foldActivity(rows, "month", Date.parse("2026-04-02T00:00:00Z"));
      expect(periods.length).toBe(4);
      for (const p of periods) expect(new Date(p).getDate()).toBe(1);
      expect(series.get("a")).toEqual([2, 0, 1, 0]);
      // 23:30Z on 31 March is 00:30 on 1 April in BST
      expect(series.get("b")).toEqual([0, 0, 0, 5]);
    });
  });
  test("weeks start on local Monday midnight across a DST change", () => {
    withZone("Europe/London", () => {
      const rows: [string, number, number][] = [
        ["a", Date.parse("2026-03-20T12:00:00Z"), 1],
        ["a", Date.parse("2026-04-03T12:00:00Z"), 1],
      ];
      const { periods } = foldActivity(rows, "week", Date.parse("2026-04-03T13:00:00Z"));
      expect(periods.length).toBe(3);
      for (const p of periods) {
        const d = new Date(p);
        expect(d.getDay()).toBe(1);
        expect(d.getHours()).toBe(0);
      }
      expect(startOfPeriod(periods[2]!, "week")).toBe(periods[2]!);
    });
  });
  test("empty input yields nothing", () => {
    expect(foldActivity([], "week").periods).toEqual([]);
  });
});

describe("milestones", () => {
  test("crossing several thresholds in one day records each", () => {
    const days = foldDays([
      [Date.parse("2026-01-01T12:00:00Z"), 50],
      [Date.parse("2026-01-02T12:00:00Z"), 1200],
    ]);
    const m = milestones(days);
    expect(m.map((x) => x.count)).toEqual([1, 100, 1000]);
    expect(m[1]!.day).toBe(days[1]!.day);
  });
});

describe("aura", () => {
  test("palette picks the vivid centre over a grey border", () => {
    const w = 20;
    const h = 20;
    const px = new Uint8ClampedArray(w * h * 4);
    for (let y = 0; y < h; y++)
      for (let x = 0; x < w; x++) {
        const i = (y * w + x) * 4;
        const centre = x > 5 && x < 14 && y > 5 && y < 14;
        px.set(centre ? [220, 40, 60, 255] : [90, 90, 92, 255], i);
      }
    const colors = selectPalette(px, w, h);
    const main = hexToOklch(colors[0]!)!;
    expect(main.c).toBeGreaterThan(0.1);
    expect(main.h < 40 || main.h > 340).toBe(true);
  });
  test("pinned beats portrait beats fallback", () => {
    expect(resolveAura(["#2dd4bf"], ["#ff0000"], 0)).toEqual(auraFrom("#2dd4bf"));
    expect(resolveAura(undefined, ["#ff0000"], 0)).toEqual(auraFrom("#ff0000"));
    expect(resolveAura(undefined, [], 7)).toEqual(auraFrom(FALLBACK[7 % FALLBACK.length]!));
  });
  test("tones keep a fixed lightness, so ink always reads on bone", () => {
    for (const hex of ["#ffff00", "#00ffff", "#ff00ff", "#2244ff", "#808080", "#c8e600"]) {
      const a = auraFrom(hex);
      expect(contrastRatio(a.ink, BONE)).toBeGreaterThanOrEqual(3);
      expect(Math.abs(hexToOklch(a.fill)!.l - 0.7)).toBeLessThan(0.02);
    }
  });
  test("a grey colour stays grey", () => {
    expect(hexToOklch(auraFrom("#808080").fill)!.c).toBeLessThan(0.01);
  });
});
