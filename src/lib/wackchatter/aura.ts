/**
 * Aura: a character's colour, the way WackChatter themes itself from a portrait.
 *
 * WackChatter keeps lightness fixed and lets the character decide only hue and how vivid it
 * is; that is what makes any colour safe to theme with. Bone is light where WackChatter is
 * dark, so the lightness steps are different, but the rule is the same: contrast is set by a
 * lightness that never moves. ECharts can't parse `oklch()`, so every tone comes out as hex.
 *
 * Palette extraction and the OKLab maths are ported from WackChatter
 * (`src/features/chat/portraitPalette.ts`, `shared/colors/color.ts`) so a character reads as
 * the same colour in both apps.
 */

export type Rgb = [number, number, number];
interface Lab {
  l: number;
  a: number;
  b: number;
}
export interface Oklch {
  l: number;
  c: number;
  h: number;
}

export function hexToRgb(hex: string): Rgb | null {
  const match = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(hex.trim());
  if (!match) return null;
  let digits = match[1]!;
  if (digits.length === 3) digits = [...digits].map((d) => d + d).join("");
  return [0, 2, 4].map((i) => Number.parseInt(digits.slice(i, i + 2), 16)) as Rgb;
}

export function rgbToHex(rgb: Rgb): string {
  return `#${rgb
    .map((n) =>
      Math.round(Math.max(0, Math.min(255, n)))
        .toString(16)
        .padStart(2, "0"),
    )
    .join("")}`;
}

const linear = (v: number) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
const gamma = (v: number) => (v <= 0.0031308 ? v * 12.92 : 1.055 * v ** (1 / 2.4) - 0.055);

function rgbToLab(rgb: Rgb): Lab {
  const [r, g, b] = rgb.map((n) => linear(n / 255)) as Rgb;
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  return {
    l: 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    a: 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    b: 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  };
}

function labToRgb({ l, a, b }: Lab): Rgb {
  const ll = (l + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const mm = (l - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const ss = (l - 0.0894841775 * a - 1.291485548 * b) ** 3;
  return [
    4.0767416621 * ll - 3.3077115913 * mm + 0.2309699292 * ss,
    -1.2684380046 * ll + 2.6097574011 * mm - 0.3413193965 * ss,
    -0.0041960863 * ll - 0.7034186147 * mm + 1.707614701 * ss,
  ].map((n) => gamma(n) * 255) as Rgb;
}

export function hexToOklch(hex: string): Oklch | null {
  const rgb = hexToRgb(hex);
  if (!rgb) return null;
  const { l, a, b } = rgbToLab(rgb);
  return { l, c: Math.hypot(a, b), h: ((Math.atan2(b, a) * 180) / Math.PI + 360) % 360 };
}

/** Reduces chroma into sRGB without rotating the hue or clipping channels. */
export function oklchToHex({ l, c, h }: Oklch): string {
  const angle = (h * Math.PI) / 180;
  const at = (chroma: number) => labToRgb({ l, a: chroma * Math.cos(angle), b: chroma * Math.sin(angle) });
  const inGamut = (rgb: Rgb) => rgb.every((n) => n >= -0.01 && n <= 255.01);
  let rgb = at(c);
  if (!inGamut(rgb)) {
    let low = 0;
    let high = c;
    for (let i = 0; i < 16; i++) {
      const mid = (low + high) / 2;
      if (inGamut(at(mid))) low = mid;
      else high = mid;
    }
    rgb = at(low);
  }
  return rgbToHex(rgb);
}

export function contrastRatio(a: string, b: string): number {
  const lum = (hex: string) => {
    const [r, g, bl] = (hexToRgb(hex) ?? [0, 0, 0]).map((n) => linear(n / 255)) as Rgb;
    return 0.2126 * r + 0.7152 * g + 0.0722 * bl;
  };
  const x = lum(a);
  const y = lum(b);
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

interface Region {
  mass: number;
  weight: number;
  edge: number;
  rgb: Rgb;
  lab: Lab;
}
const distance = (a: Lab, b: Lab) => Math.hypot(a.l - b.l, a.a - b.a, a.b - b.b);
function mergeRegion(target: Region, other: Region) {
  const mass = target.mass + other.mass;
  target.rgb = target.rgb.map((n, i) => (n * target.mass + other.rgb[i]! * other.mass) / mass) as Rgb;
  target.mass = mass;
  target.weight += other.weight;
  target.edge += other.edge;
  target.lab = rgbToLab(target.rgb);
}

/** Up to three distinct portrait colours, main colour first; centre-weighted, scenery-damped. */
export function selectPalette(pixels: Uint8ClampedArray, width: number, height: number): string[] {
  const histogram = new Map<number, Region>();
  let total = 0;
  for (let offset = 0; offset + 3 < pixels.length; offset += 4) {
    const alpha = pixels[offset + 3]! / 255;
    if (alpha < 0.125) continue;
    const rgb: Rgb = [pixels[offset]!, pixels[offset + 1]!, pixels[offset + 2]!];
    const key = ((rgb[0] >> 4) << 8) | ((rgb[1] >> 4) << 4) | (rgb[2] >> 4);
    const x = ((offset / 4) % width) / Math.max(1, width - 1);
    const y = Math.floor(offset / 4 / width) / Math.max(1, height - 1);
    const radius = (x * 2 - 1) ** 2 + ((y - 0.46) * 2) ** 2;
    const weight = alpha * (0.25 + Math.exp(-2.5 * radius));
    const edge = x < 0.14 || x > 0.86 || y < 0.1 || y > 0.9 ? alpha : 0;
    const region: Region = { mass: alpha, weight, edge, rgb, lab: { l: 0, a: 0, b: 0 } };
    const existing = histogram.get(key);
    if (existing) mergeRegion(existing, region);
    else histogram.set(key, region);
    total += alpha;
  }
  if (!total) return [];
  const seeds = [...histogram].sort((a, b) => b[1].mass - a[1].mass || a[0] - b[0]);
  const clusters: Region[] = [];
  for (const [, region] of seeds) {
    region.lab = rgbToLab(region.rgb);
    let nearest: Region | undefined;
    let delta = Number.POSITIVE_INFINITY;
    for (const cluster of clusters) {
      const next = distance(region.lab, cluster.lab);
      if (next < delta) {
        delta = next;
        nearest = cluster;
      }
    }
    if (nearest && (delta < 0.065 || clusters.length >= 24)) mergeRegion(nearest, region);
    else clusters.push(region);
  }
  const scored = clusters
    .filter((r) => r.mass >= Math.max(1, total * 0.0125))
    .map((r) => {
      const chroma = Math.hypot(r.lab.a, r.lab.b);
      const edgeWeight = Math.max(0.18, 1 - (1.6 * r.edge) / r.mass);
      return { r, score: Math.sqrt(r.weight) * (0.18 + 4 * chroma) * edgeWeight };
    })
    .sort((a, b) => b.score - a.score || rgbToHex(a.r.rgb).localeCompare(rgbToHex(b.r.rgb)));
  const candidates: Region[] = [];
  for (const { r } of scored) {
    if (candidates.every((o) => distance(r.lab, o.lab) >= 0.075)) candidates.push(r);
    if (candidates.length === 6) break;
  }
  const colors: string[] = [];
  for (const region of candidates) {
    const color = rgbToHex(region.rgb);
    const lch = hexToOklch(color)!;
    const distinct = colors.every((other) => {
      const prev = hexToOklch(other)!;
      if (lch.c < 0.025 && prev.c < 0.025) return false;
      const hue = Math.abs(((lch.h - prev.h + 540) % 360) - 180);
      return hue >= 20 || Math.abs(lch.c - prev.c) >= 0.055;
    });
    if (distinct) colors.push(color);
    if (colors.length === 3) break;
  }
  return colors;
}

export interface Aura {
  /** Block fills, chart series. */
  fill: string;
  /** Text and rules on bone: dark enough to read. */
  ink: string;
  /** Page tint behind a profile. */
  wash: string;
}

const FULL_CHROMA = 0.14;

/** Fixed-lightness tones of a colour; a grey colour stays grey. */
export function auraFrom(hex: string): Aura {
  const lch = hexToOklch(hex) ?? { l: 0.6, c: 0, h: 0 };
  const vivid = Math.min(1, lch.c / 0.12);
  const tone = (l: number, c: number) => oklchToHex({ l, c: c * vivid, h: lch.h });
  return { fill: tone(0.7, FULL_CHROMA), ink: tone(0.42, FULL_CHROMA * 0.85), wash: tone(0.895, 0.022) };
}

/** Distinct fallbacks for characters with no portrait and no pinned colour. */
export const FALLBACK = ["#00c2c2", "#c8e600", "#ff1f6f", "#7c5cff", "#ff4d00", "#3d8eff"];

/** Pinned colours win, then the portrait, then a fallback picked by the character's own hash. */
export function resolveAura(pinned: string[] | undefined, portrait: string[] | undefined, seed: number): Aura {
  const source = pinned?.[0] ?? portrait?.[0] ?? FALLBACK[seed % FALLBACK.length]!;
  return auraFrom(source);
}

const cache = new Map<string, Promise<string[]>>();

/** Palette of an image data URL, extracted once per key on a 96px canvas. */
export function portraitPalette(key: string, src: string): Promise<string[]> {
  let hit = cache.get(key);
  if (!hit) {
    hit = new Promise<string[]>((resolve) => {
      const img = new Image();
      img.onload = () => {
        const scale = Math.min(1, 96 / Math.max(img.naturalWidth, img.naturalHeight, 1));
        const canvas = document.createElement("canvas");
        canvas.width = Math.max(1, Math.round(img.naturalWidth * scale));
        canvas.height = Math.max(1, Math.round(img.naturalHeight * scale));
        const ctx = canvas.getContext("2d", { willReadFrequently: true });
        if (!ctx) return resolve([]);
        ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
        resolve(selectPalette(ctx.getImageData(0, 0, canvas.width, canvas.height).data, canvas.width, canvas.height));
      };
      img.onerror = () => resolve([]);
      img.src = src;
    });
    cache.set(key, hit);
  }
  return hit;
}
