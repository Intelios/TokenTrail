import { describe, expect, test } from "bun:test";
import type { HourBucket } from "../src/lib/wackchatter/buckets";
import { busiestDay, foldDays, startOfDay, streaks } from "../src/lib/wackchatter/buckets";

/** A UTC-hour bucket, as the server sends them. */
function hour(iso: string, count = 1): HourBucket {
  return [Date.parse(iso), count];
}

function isoDay(ms: number): string {
  const date = new Date(ms);
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(
    date.getDate(),
  ).padStart(2, "0")}`;
}

describe("folding into local days", () => {
  test("hours in the same local day merge", () => {
    const days = foldDays([
      hour("2026-08-08T15:00:00Z", 3),
      hour("2026-08-08T16:00:00Z", 4),
    ]);
    expect(days).toHaveLength(1);
    expect(days[0]?.count).toBe(7);
  });

  test("a late-evening hour lands on the local day, not the UTC one", () => {
    // 23:30 local is tomorrow in UTC anywhere east of Greenwich, and yesterday in UTC
    // anywhere west of it. Either way the session belongs to the day the user lived it.
    const local = new Date(2026, 6, 15, 23, 30, 0);
    const days = foldDays([[local.getTime(), 1]]);
    expect(days).toHaveLength(1);
    expect(isoDay(days[0]!.day)).toBe("2026-07-15");
  });

  test("days come back ascending", () => {
    const days = foldDays([
      hour("2026-08-10T12:00:00Z"),
      hour("2026-08-08T12:00:00Z"),
    ]);
    expect(days.map((entry) => entry.day)).toEqual(
      [...days.map((e) => e.day)].sort((a, b) => a - b),
    );
  });
});

describe("streaks", () => {
  const day = (offset: number, count = 1) => {
    const date = new Date(2026, 7, 20, 12, 0, 0);
    date.setDate(date.getDate() + offset);
    return { day: startOfDay(date.getTime()), count };
  };
  const now = new Date(2026, 7, 20, 12, 0, 0).getTime();

  test("counts an unbroken run ending today", () => {
    expect(streaks([day(-2), day(-1), day(0)], now)).toEqual({
      current: 3,
      longest: 3,
    });
  });

  test("a run ending yesterday is still current", () => {
    // Otherwise the streak reads zero every morning until the first message of the day.
    expect(streaks([day(-2), day(-1)], now).current).toBe(2);
  });

  test("a run ending two days ago has lapsed", () => {
    expect(streaks([day(-3), day(-2)], now)).toEqual({
      current: 0,
      longest: 2,
    });
  });

  test("the longest run survives a later break", () => {
    expect(streaks([day(-9), day(-8), day(-7), day(-6), day(-1)], now)).toEqual(
      {
        current: 1,
        longest: 4,
      },
    );
  });

  test("empty days are not part of a run", () => {
    expect(streaks([day(-2), day(-1, 0), day(0)], now)).toEqual({
      current: 1,
      longest: 1,
    });
  });

  test("no activity is no streak", () => {
    expect(streaks([], now)).toEqual({ current: 0, longest: 0 });
  });
});

describe("summaries", () => {
  test("busiestDay picks the peak, and null when there is none", () => {
    const days = foldDays([
      [new Date(2026, 7, 18, 12).getTime(), 4],
      [new Date(2026, 7, 19, 12).getTime(), 9],
    ]);
    expect(busiestDay(days)?.count).toBe(9);
    expect(busiestDay([])).toBeNull();
  });
});

// Local Date applies historical offsets and fractional-hour zones to each instant.
test("local midnight survives fractional-hour offsets", () => {
  const previous = process.env.TZ;
  try {
    process.env.TZ = "Asia/Kathmandu";
    const days = foldDays([
      hour("2026-10-10T18:14:00Z"),
      hour("2026-10-10T18:16:00Z"),
    ]);
    expect(days.map((d) => isoDay(d.day))).toEqual([
      "2026-10-10",
      "2026-10-11",
    ]);
  } finally {
    if (previous === undefined) delete process.env.TZ;
    else process.env.TZ = previous;
  }
});
