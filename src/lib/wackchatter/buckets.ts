/**
 * Calendar folding.
 *
 * The server sends timestamp/count buckets without rounding away minutes, and every calendar question is answered
 * here. That split is deliberate: `send_date` is UTC and so is SQLite's `date()`, so
 * bucketing UTC hours or days server-side would file a 23:30 session under tomorrow, and sending a
 * fixed client offset instead would be wrong on the far side of a daylight-saving change.
 * `new Date(ms)` resolves each instant against the rules in force at that instant, which
 * is the only version of this that is right all year.
 */

export type HourBucket = [number, number];

export interface DayBucket {
  /** Local midnight, as epoch ms. */
  day: number;
  count: number;
}

/** Local midnight for the day containing this instant. */
export function startOfDay(ms: number): number {
  const date = new Date(ms);
  date.setHours(0, 0, 0, 0);
  return date.getTime();
}

/** Ascending by day, gaps omitted. */
export function foldDays(hours: readonly HourBucket[]): DayBucket[] {
  const totals = new Map<number, number>();
  for (const [hour, count] of hours) {
    const day = startOfDay(hour);
    totals.set(day, (totals.get(day) ?? 0) + count);
  }
  return [...totals.entries()]
    .map(([day, count]) => ({ day, count }))
    .sort((left, right) => left.day - right.day);
}

export interface Streaks {
  /** Days up to and including today, or up to yesterday if today is still empty. */
  current: number;
  longest: number;
}

/**
 * Today counting as a break the moment midnight passes would report a streak of zero every
 * morning, so an unbroken run ending yesterday is still current until today ends.
 */
export function streaks(
  days: readonly DayBucket[],
  now: number = Date.now(),
): Streaks {
  const active = days
    .filter((entry) => entry.count > 0)
    .map((entry) => entry.day);
  if (active.length === 0) return { current: 0, longest: 0 };

  let longest = 1;
  let run = 1;
  for (let index = 1; index < active.length; index += 1) {
    const previous = active[index - 1];
    const day = active[index];
    if (previous === undefined || day === undefined) continue;
    // Same calendar-day step, for the same daylight-saving reason.
    const expected = new Date(previous);
    expected.setDate(expected.getDate() + 1);
    run = day === startOfDay(expected.getTime()) ? run + 1 : 1;
    longest = Math.max(longest, run);
  }

  const today = startOfDay(now);
  const yesterday = new Date(today);
  yesterday.setDate(yesterday.getDate() - 1);
  const last = active[active.length - 1];
  const live = last === today || last === startOfDay(yesterday.getTime());

  return { current: live ? run : 0, longest };
}

export function busiestDay(days: readonly DayBucket[]): DayBucket | null {
  let best: DayBucket | null = null;
  for (const entry of days) {
    if (entry.count > 0 && (best === null || entry.count > best.count))
      best = entry;
  }
  return best;
}

/** 7×24 counts: `[weekday][hour]`, weekday 0 is Monday, both local. */
export function foldWeekHour(hours: readonly HourBucket[]): number[][] {
  const grid = Array.from({ length: 7 }, () => new Array<number>(24).fill(0));
  for (const [instant, count] of hours) {
    const date = new Date(instant);
    grid[(date.getDay() + 6) % 7]![date.getHours()]! += count;
  }
  return grid;
}

export type Period = "week" | "month";

/** Local start of the week (Monday) or month containing this instant. */
export function startOfPeriod(ms: number, period: Period): number {
  const date = new Date(ms);
  date.setHours(0, 0, 0, 0);
  if (period === "month") date.setDate(1);
  else date.setDate(date.getDate() - ((date.getDay() + 6) % 7));
  return date.getTime();
}

export interface PeriodSeries {
  /** Contiguous period starts, ascending. */
  periods: number[];
  /** Per id, one count per period. */
  series: Map<string, number[]>;
}

/**
 * Per-id counts over a contiguous run of local weeks or months. Stepping by calendar
 * unit rather than fixed milliseconds keeps every boundary on local midnight across
 * daylight-saving changes.
 */
export function foldActivity(
  rows: readonly (readonly [string, number, number])[],
  period: Period,
  now: number = Date.now(),
): PeriodSeries {
  if (!rows.length) return { periods: [], series: new Map() };
  let first = Number.POSITIVE_INFINITY;
  for (const row of rows) first = Math.min(first, row[1]);
  const periods: number[] = [];
  const cursor = new Date(startOfPeriod(first, period));
  const last = startOfPeriod(now, period);
  while (cursor.getTime() <= last) {
    periods.push(cursor.getTime());
    if (period === "month") cursor.setMonth(cursor.getMonth() + 1);
    else cursor.setDate(cursor.getDate() + 7);
  }
  const index = new Map(periods.map((p, i) => [p, i]));
  const series = new Map<string, number[]>();
  for (const [id, instant, count] of rows) {
    const at = index.get(startOfPeriod(instant, period));
    if (at === undefined) continue;
    let values = series.get(id);
    if (!values) series.set(id, (values = new Array<number>(periods.length).fill(0)));
    values[at]! += count;
  }
  return { periods, series };
}

export interface Milestone {
  /** Local day it was reached. */
  day: number;
  /** The reply number, 1 for the first. */
  count: number;
}

/** First reply and every power-of-ten-ish threshold the running total crosses. */
export function milestones(
  days: readonly DayBucket[],
  thresholds: readonly number[] = [1, 100, 1000, 10000, 100000],
): Milestone[] {
  const out: Milestone[] = [];
  let total = 0;
  let next = 0;
  for (const entry of days) {
    total += entry.count;
    while (next < thresholds.length && total >= thresholds[next]!) {
      out.push({ day: entry.day, count: thresholds[next]! });
      next += 1;
    }
  }
  return out;
}
