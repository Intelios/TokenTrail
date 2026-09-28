/// Usage Wrapped — the shareable summary card, drawn with Canvas 2D in the
/// Marathon · Bone system: bone ground, 2px ink rules, hard accent blocks,
/// zero radius. One draw routine serves both the settings preview and the
/// exported PNG, so the preview is literally the file that gets saved.

import type { WrappedSummary } from '$lib/api';
import { flatColor } from '$lib/chartTheme';
import {
  basename,
  familyFor,
  fmtTokens,
  fmtTokensSplit,
  modelFlat,
  sourceColor,
  sourceLabel,
} from '$lib/format';

export const WRAPPED_W = 1080;
export const WRAPPED_H = 1350;

const UI = '"Inter Tight", system-ui, sans-serif';
const MONO = '"IBM Plex Mono", ui-monospace, monospace';
const DISP = '"Anton", "Inter Tight", sans-serif';

const MONTHS = ['JAN', 'FEB', 'MAR', 'APR', 'MAY', 'JUN', 'JUL', 'AUG', 'SEP', 'OCT', 'NOV', 'DEC'];

/** Marathon palette, resolved from the app.css tokens at draw time so the
 *  card always matches the theme it is previewed in. */
function palette() {
  const root = typeof document !== 'undefined' ? getComputedStyle(document.documentElement) : null;
  const v = (name: string, fallback: string) => root?.getPropertyValue(name).trim() || fallback;
  return {
    bone: v('--bone', '#e8e4d9'),
    ink: v('--ink', '#0d0d0b'),
    org: v('--org', '#ff4d00'),
    cyn: v('--cyn', '#00c2c2'),
    acd: v('--acd', '#c8e600'),
    mag: v('--mag', '#ff1f6f'),
    vio: v('--vio', '#7c5cff'),
    dim: v('--dim', 'rgba(13,13,11,0.55)'),
    hair: v('--hair', 'rgba(13,13,11,0.16)'),
  };
}

/** Tracked (letter-spaced) text drawn per character — identical in every
 *  webview, unlike the canvas `letterSpacing` property. Alignment is forced
 *  to left so the advance math holds even when the caller centered text. */
function trackedText(
  ctx: CanvasRenderingContext2D,
  text: string,
  x: number,
  y: number,
  spacing: number,
) {
  const prev = ctx.textAlign;
  ctx.textAlign = 'left';
  let cx = x;
  for (const ch of text) {
    ctx.fillText(ch, cx, y);
    cx += ctx.measureText(ch).width + spacing;
  }
  ctx.textAlign = prev;
}

function trackedWidth(ctx: CanvasRenderingContext2D, text: string, spacing: number): number {
  let w = 0;
  for (const ch of text) w += ctx.measureText(ch).width + spacing;
  return Math.max(0, w - spacing);
}

/** Truncate with an ellipsis so the string fits `maxW` at current font. */
function fitText(ctx: CanvasRenderingContext2D, text: string, maxW: number): string {
  if (ctx.measureText(text).width <= maxW) return text;
  let out = text;
  while (out.length > 1 && ctx.measureText(out + '…').width > maxW) out = out.slice(0, -1);
  return out + '…';
}

/** "2026-03-14" → "MAR 14" in UTC, matching how the aggregates bucket dates. */
function fmtDay(iso: string): string {
  const d = new Date(iso + 'T00:00:00Z');
  if (Number.isNaN(d.getTime())) return iso;
  return `${MONTHS[d.getUTCMonth()]} ${d.getUTCDate()}`;
}

/** The persona headline, first match wins. Ranks are chosen so a normal
 *  month still earns something fun — the family fallback always fires. */
export function wrappedTitle(s: WrappedSummary): string {
  if (s.total_tokens <= 0) return 'NOTHING BURNED YET';
  const window = Math.max(1, s.window_days);
  const top = s.top_models[0];
  const family = top ? familyFor(top.model) : 'Other';
  const brand = family === 'Other' ? 'TOKEN' : family.toUpperCase();
  if (window >= 7 && s.active_days >= window * 0.97) return 'NO DAYS OFF';
  if (s.longest_streak >= 30) return 'MARATHON RUNNER';
  if (s.night_share >= 0.3) return 'NIGHT SHIFT';
  if (top && top.tokens >= s.total_tokens * 0.7) return `${brand} DEVOTEE`;
  if (s.by_source.filter((x) => x.tokens > 0).length >= 4) return 'MULTI-HARNESS OPERATOR';
  return `${brand} AFICIONADO`;
}

/** Up to two facts for the ink callout band, most interesting first. */
export function wrappedCallouts(s: WrappedSummary): string[] {
  const out: string[] = [];
  if (s.peak_day && s.peak_day_tokens > 0) {
    out.push(`${fmtDay(s.peak_day)} WAS YOUR BIGGEST DAY — ${fmtTokens(s.peak_day_tokens)} TOKENS.`);
  }
  if (s.busiest_hour != null) {
    out.push(`${String(s.busiest_hour).padStart(2, '0')}:00 IS YOUR POWER HOUR.`);
  }
  if (s.top_project) {
    out.push(`MOST OF IT WENT TO ${basename(s.top_project).toUpperCase()}.`);
  }
  return out.slice(0, 2);
}

/** One value per waveform column: calendar days for week/month (zeros kept,
 *  anchored on the last day with data), ~52 buckets across the whole span for
 *  all-time where per-day columns would be unreadably thin. */
function waveformBuckets(s: WrappedSummary): number[] {
  const cells = s.daily;
  if (!cells.length) return [];
  if (s.period_days > 0) {
    const byDate = new Map(cells.map((c) => [c.date, c.tokens]));
    const last = new Date(cells[cells.length - 1].date + 'T00:00:00Z').getTime();
    const out: number[] = [];
    for (let i = s.period_days - 1; i >= 0; i--) {
      const d = new Date(last - i * 86_400_000).toISOString().slice(0, 10);
      out.push(byDate.get(d) ?? 0);
    }
    return out;
  }
  const n = Math.min(52, cells.length);
  const out = new Array<number>(n).fill(0);
  cells.forEach((c, i) => {
    out[Math.min(n - 1, Math.floor((i / cells.length) * n))] += c.tokens;
  });
  return out;
}

/** Render the card onto `canvas` at full export resolution (1080×1350). */
export function drawWrapped(
  canvas: HTMLCanvasElement,
  s: WrappedSummary,
  periodLabel: string,
): void {
  canvas.width = WRAPPED_W;
  canvas.height = WRAPPED_H;
  const ctx = canvas.getContext('2d');
  if (!ctx) return;
  const p = palette();
  const W = WRAPPED_W;
  const H = WRAPPED_H;
  const MX = 64;
  const CW = W - 2 * MX;
  const RX = W - MX;

  ctx.textAlign = 'left';
  ctx.textBaseline = 'alphabetic';

  // ground + frame + registration marks
  ctx.fillStyle = p.bone;
  ctx.fillRect(0, 0, W, H);
  ctx.strokeStyle = p.ink;
  ctx.lineWidth = 4;
  ctx.strokeRect(18, 18, W - 36, H - 36);
  ctx.strokeStyle = p.org;
  ctx.lineWidth = 2;
  for (const [cx, cy] of [
    [18, 18],
    [W - 18, 18],
    [18, H - 18],
    [W - 18, H - 18],
  ]) {
    ctx.strokeRect(cx - 5, cy - 5, 10, 10);
  }

  // masthead: brand left, ink period pill right
  ctx.fillStyle = p.ink;
  ctx.font = `700 24px ${UI}`;
  trackedText(ctx, 'TOKENTRAIL ★ USAGE WRAPPED', MX, 92, 2.5);
  ctx.font = `600 16px ${MONO}`;
  const pillText = periodLabel.toUpperCase();
  const pillW = trackedWidth(ctx, pillText, 1.5) + 44;
  ctx.fillStyle = p.ink;
  ctx.fillRect(RX - pillW, 64, pillW, 36);
  ctx.fillStyle = p.bone;
  trackedText(ctx, pillText, RX - pillW + 22, 88, 1.5);
  ctx.fillStyle = p.ink;
  ctx.fillRect(MX, 116, CW, 2);

  if (s.total_tokens <= 0) {
    ctx.textAlign = 'center';
    ctx.font = `400 52px ${DISP}`;
    ctx.fillText('NO USAGE RECORDED', W / 2, H / 2 - 8);
    ctx.font = `400 16px ${MONO}`;
    ctx.fillStyle = p.dim;
    trackedText(
      ctx,
      'IN THIS PERIOD — COME BACK AFTER YOUR NEXT SESSION',
      W / 2 - trackedWidth(ctx, 'IN THIS PERIOD — COME BACK AFTER YOUR NEXT SESSION', 1) / 2,
      H / 2 + 34,
      1,
    );
    return;
  }

  // persona title, auto-shrunk to hold one line
  const title = wrappedTitle(s);
  let titleSize = 62;
  ctx.font = `400 ${titleSize}px ${DISP}`;
  while (ctx.measureText(title).width > CW && titleSize > 40) {
    titleSize -= 2;
    ctx.font = `400 ${titleSize}px ${DISP}`;
  }
  ctx.fillStyle = p.ink;
  ctx.fillText(title, MX, 192);

  // hero: total tokens with the magnitude unit in orange
  const hero = fmtTokensSplit(s.total_tokens);
  ctx.font = `400 168px ${DISP}`;
  const valueW = ctx.measureText(hero.value).width;
  ctx.fillText(hero.value, MX, 348);
  if (hero.unit) {
    ctx.font = `400 64px ${DISP}`;
    ctx.fillStyle = p.org;
    ctx.fillText(hero.unit, MX + valueW + 10, 348);
    ctx.fillStyle = p.ink;
  }
  ctx.font = `500 19px ${MONO}`;
  trackedText(ctx, `TOKENS ACROSS ${s.events.toLocaleString()} GENERATIONS`, MX, 388, 1.5);

  // hero right block: the model that carried the window
  const top = s.top_models[0];
  if (top) {
    ctx.textAlign = 'right';
    ctx.font = `500 14px ${MONO}`;
    ctx.fillStyle = p.dim;
    const t1 = 'TOP MODEL';
    trackedText(ctx, t1, RX - trackedWidth(ctx, t1, 1.5), 262, 1.5);
    ctx.fillStyle = p.ink;
    ctx.font = `700 32px ${UI}`;
    ctx.fillText(fitText(ctx, top.model, CW - valueW - 260), RX, 302);
    ctx.font = `400 15px ${MONO}`;
    ctx.fillStyle = p.dim;
    ctx.fillText(
      `${Math.round((top.tokens / s.total_tokens) * 100)}% OF ALL TOKENS`,
      RX,
      332,
    );
    ctx.textAlign = 'left';
  }

  // ink callout band
  const callouts = wrappedCallouts(s);
  if (callouts.length) {
    const bandTop = 414;
    const bandH = 104;
    ctx.fillStyle = p.ink;
    ctx.fillRect(MX, bandTop, CW, bandH);
    ctx.fillStyle = p.bone;
    ctx.font = `600 23px ${UI}`;
    if (callouts.length === 1) {
      ctx.fillText(fitText(ctx, callouts[0], CW - 56), MX + 28, bandTop + 62);
    } else {
      ctx.fillText(fitText(ctx, callouts[0], CW - 56), MX + 28, bandTop + 44);
      ctx.fillText(fitText(ctx, callouts[1], CW - 56), MX + 28, bandTop + 82);
    }
  }

  // most used models board
  ctx.fillStyle = p.ink;
  ctx.font = `700 17px ${UI}`;
  trackedText(ctx, 'MOST USED MODELS', MX, 562, 2.5);
  ctx.fillStyle = p.hair;
  const hdrW = trackedWidth(ctx, 'MOST USED MODELS', 2.5);
  ctx.fillRect(MX + hdrW + 16, 556, CW - hdrW - 16, 2);

  const ACCENTS: Array<[string, string]> = [
    [p.org, '#ffffff'],
    [p.cyn, '#ffffff'],
    [p.acd, p.ink],
    [p.mag, '#ffffff'],
    [p.vio, '#ffffff'],
  ];
  const models = s.top_models.slice(0, 5);
  const topTokens = models[0]?.tokens ?? 1;
  models.forEach((m, i) => {
    const top2 = 588 + i * 68;
    const [fill, chipInk] = ACCENTS[i % ACCENTS.length];
    ctx.fillStyle = fill;
    ctx.fillRect(MX, top2 + 4, 40, 40);
    ctx.fillStyle = chipInk;
    ctx.font = `400 20px ${DISP}`;
    ctx.textAlign = 'center';
    ctx.fillText(String(i + 1), MX + 20, top2 + 32);
    ctx.textAlign = 'left';
    ctx.fillStyle = p.ink;
    ctx.font = `600 23px ${UI}`;
    ctx.fillText(fitText(ctx, m.model, CW - 56 - 220), MX + 56, top2 + 28);
    ctx.textAlign = 'right';
    ctx.font = `500 18px ${MONO}`;
    ctx.fillText(fmtTokens(m.tokens), RX, top2 + 28);
    ctx.textAlign = 'left';
    // share bar, filled relative to the leader like the dashboard rankbars
    ctx.fillStyle = p.hair;
    ctx.fillRect(MX + 56, top2 + 40, CW - 56, 6);
    ctx.fillStyle = modelFlat(m.model, i);
    ctx.fillRect(MX + 56, top2 + 40, ((CW - 56) * m.tokens) / topTokens, 6);
    ctx.textAlign = 'right';
    ctx.font = `400 13px ${MONO}`;
    ctx.fillStyle = p.dim;
    ctx.fillText(`${Math.round((m.tokens / s.total_tokens) * 100)}%`, RX, top2 + 62);
    ctx.textAlign = 'left';
  });

  // harness mix: segmented block bar + legend
  const srcTop = 588 + Math.max(models.length, 1) * 68 + 34;
  ctx.fillStyle = p.ink;
  ctx.font = `700 17px ${UI}`;
  trackedText(ctx, 'HARNESS MIX', MX, srcTop, 2.5);
  const srcHdrW = trackedWidth(ctx, 'HARNESS MIX', 2.5);
  ctx.fillStyle = p.hair;
  ctx.fillRect(MX + srcHdrW + 16, srcTop - 6, CW - srcHdrW - 16, 2);

  const barTop = srcTop + 18;
  const barH = 30;
  const sources = s.by_source.filter((x) => x.tokens > 0);
  let sx = MX;
  for (const src of sources) {
    const w = (CW * src.tokens) / s.total_tokens;
    ctx.fillStyle = flatColor(sourceColor(src.source));
    ctx.fillRect(sx, barTop, w, barH);
    sx += w;
  }
  ctx.strokeStyle = p.ink;
  ctx.lineWidth = 2;
  ctx.strokeRect(MX, barTop, CW, barH);

  ctx.font = `400 14px ${MONO}`;
  ctx.fillStyle = p.dim;
  const shown = sources.slice(0, 4);
  const rest = sources.slice(4).reduce((a, x) => a + x.tokens, 0);
  const legendParts = shown.map(
    (x) => `${sourceLabel(x.source).toUpperCase()} ${Math.round((x.tokens / s.total_tokens) * 100)}%`,
  );
  if (rest > 0) legendParts.push(`OTHER ${Math.round((rest / s.total_tokens) * 100)}%`);
  ctx.fillText(fitText(ctx, legendParts.join('  ·  '), CW), MX, barTop + barH + 30);

  // stat strip: four cells between 2px rules
  const stripTop = barTop + barH + 56;
  const stripH = 96;
  ctx.fillStyle = p.ink;
  ctx.fillRect(MX, stripTop, CW, 2);
  ctx.fillRect(MX, stripTop + stripH, CW, 2);
  const cells: Array<[string, string]> = [
    [s.active_days.toLocaleString(), 'ACTIVE DAYS'],
    [`${s.longest_streak}D`, 'LONGEST STREAK'],
    [s.sessions.toLocaleString(), 'SESSIONS'],
    [s.busiest_hour != null ? `${String(s.busiest_hour).padStart(2, '0')}:00` : '—', 'PEAK HOUR'],
  ];
  const cellW = CW / cells.length;
  ctx.textAlign = 'center';
  cells.forEach(([num, label], i) => {
    const cx = MX + cellW * i + cellW / 2;
    if (i > 0) {
      ctx.fillStyle = p.hair;
      ctx.fillRect(MX + cellW * i, stripTop + 12, 2, stripH - 24);
    }
    ctx.fillStyle = p.ink;
    ctx.font = `400 40px ${DISP}`;
    ctx.fillText(num, cx, stripTop + 56);
    ctx.fillStyle = p.dim;
    ctx.font = `500 12px ${MONO}`;
    const lw = trackedWidth(ctx, label, 2);
    trackedText(ctx, label, cx - lw / 2, stripTop + 80, 2);
  });
  ctx.textAlign = 'left';

  // daily burn waveform
  const waveBase = stripTop + stripH + 108;
  const buckets = waveformBuckets(s);
  if (buckets.length) {
    const max = Math.max(...buckets, 1);
    const gap = 3;
    const colW = Math.min(26, (CW - gap * (buckets.length - 1)) / buckets.length);
    const totalW = colW * buckets.length + gap * (buckets.length - 1);
    let bx = MX;
    ctx.fillStyle = p.ink;
    for (const v of buckets) {
      const h = 4 + (v / max) * 88;
      ctx.fillRect(bx, waveBase - h, colW, h);
      bx += colW + gap;
    }
    // waveform rule runs only under the columns, not the empty right margin
    ctx.fillRect(MX, waveBase, Math.max(totalW, CW * 0.4), 2);
  } else {
    ctx.fillStyle = p.hair;
    ctx.fillRect(MX, waveBase, CW, 2);
  }

  // footer: generation stamp left; estimated share flagged right, never
  // left to pass as measured
  ctx.font = `400 13px ${MONO}`;
  ctx.fillStyle = p.dim;
  trackedText(ctx, `GENERATED ${new Date().toISOString().slice(0, 10)} · TOKENTRAIL`, MX, 1318, 1);
  if (s.estimated_tokens > 0) {
    const est = `*INCL ${fmtTokens(s.estimated_tokens)} EST. TOKENS (SELF-REPORTED)`;
    ctx.textAlign = 'right';
    ctx.fillText(est, RX, 1318);
    ctx.textAlign = 'left';
  }
}
