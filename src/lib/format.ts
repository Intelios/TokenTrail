import { linearGrad, cssColor, flatColor, type ChartColor } from './chartTheme';

/// Brand-locked palettes — each harness and model family wears its maker's
/// actual brand color. Flat values are hex strings; gradients (Gemini) are
/// ECharts specs. Use the *Swatch helpers for DOM and *Flat for SVG strokes.

/** Gemini sparkle sweep — shared by the Gemini family and Antigravity. */
export const GEMINI_STOPS = ['#4796E3', '#9177C7', '#CA6673'];

export const SOURCE_COLORS: Record<string, ChartColor> = {
  claude_code: '#D97757',
  codex: '#10A37F',
  zcode: '#7c5cff',
  antigravity: linearGrad(GEMINI_STOPS),
  opencode: '#ff1f6f',
  gemini: '#4796E3',
  wackchatter: '#c2ee4a',
};

export const SOURCE_LABEL: Record<string, string> = {
  zcode: 'ZCode',
  claude_code: 'Claude Code',
  codex: 'Codex',
  opencode: 'OpenCode',
  gemini: 'Gemini CLI',
  antigravity: 'Antigravity',
  wackchatter: 'WackChatter',
};

/// Default palette for models with no brand family — TokenTrail's own
/// Marathon accents (cyan, acid, magenta, violet; orange/blue/green zones
/// belong to brands), cycling by rank, then ink-shades for the tail.
export const MODEL_PALETTE = [
  '#00c2c2', '#c8e600', '#ff1f6f', '#7c5cff', '#8a8578', '#4a473e',
];

/// Input / Output / Cache read / Cache write — shared by charts across pages.
export const MIX_COLORS = ['#3d8eff', '#00c2c2', '#c8e600', '#ff1f6f'];

/** Marathon violet accent for thinking / reasoning tokens. */
export const REASONING_COLOR = '#7c5cff';

export function sourceColor(s: string): ChartColor {
  return SOURCE_COLORS[s] ?? '#8a8578';
}

/** CSS background value for DOM swatches — turns gradients into linear-gradient(). */
export function sourceSwatch(s: string): string {
  return cssColor(sourceColor(s));
}

export function sourceLabel(s: string): string {
  return SOURCE_LABEL[s] ?? s;
}

export function fmtTokens(n: number): string {
  if (!isFinite(n)) return '—';
  if (n >= 1e9) return (n / 1e9).toFixed(n >= 1e10 ? 0 : 1) + 'B';
  if (n >= 1e6) return (n / 1e6).toFixed(n >= 1e7 ? 0 : 1) + 'M';
  if (n >= 1e3) return (n / 1e3).toFixed(n >= 1e4 ? 0 : 1) + 'K';
  return String(Math.round(n));
}

/// fmtTokens with the magnitude unit split out, so hero numerals can render
/// the suffix in a different style (orange "M" in "141M").
export function fmtTokensSplit(n: number): { value: string; unit: string } {
  if (!isFinite(n)) return { value: '—', unit: '' };
  if (n >= 1e9) return { value: (n / 1e9).toFixed(n >= 1e10 ? 0 : 1), unit: 'B' };
  if (n >= 1e6) return { value: (n / 1e6).toFixed(n >= 1e7 ? 0 : 1), unit: 'M' };
  if (n >= 1e3) return { value: (n / 1e3).toFixed(n >= 1e4 ? 0 : 1), unit: 'K' };
  return { value: String(Math.round(n)), unit: '' };
}

export function fmtCost(n: number | null): string {
  if (n == null || !isFinite(n)) return '—';
  if (n >= 1000) return '$' + Math.round(n).toLocaleString();
  if (n >= 1) return '$' + n.toFixed(2);
  if (n >= 0.01) return '$' + n.toFixed(2);
  return '$' + n.toFixed(4);
}

export function fmtDate(ts: number | null): string {
  if (!ts) return '—';
  return new Date(ts).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' });
}

export function fmtDateTime(ts: number | null): string {
  if (!ts) return '—';
  return new Date(ts).toLocaleString(undefined, { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' });
}

export function basename(p: string): string {
  const parts = p.split('/').filter(Boolean);
  return parts[parts.length - 1] ?? p;
}

/// Provider brand colors per model family — keys must stay in sync with the
/// FAMILY_RULES prefixes in src-tauri/src/families.rs. Gemini wears its
/// sparkle gradient; GLM/Grok keep monochrome ink tones (their brand is ink).
/// "Other" is deliberately absent: unbranded models cycle MODEL_PALETTE by
/// rank in familyColor() instead of sharing one flat gray.
export const FAMILY_COLORS: Record<string, ChartColor> = {
  Claude: '#D97757',
  GPT: '#10A37F',
  Gemini: linearGrad(GEMINI_STOPS),
  DeepSeek: '#4D6BFE',
  Meta: '#0064E0',
  Mistral: '#CC5500',
  Kimi: '#007CFF',
  Qwen: '#615CED',
  GLM: '#6e6a5e',
  Grok: '#0d0d0b',
  MiMo: '#FF6900',
};

export function familyColor(family: string, rank: number): ChartColor {
  return FAMILY_COLORS[family] ?? MODEL_PALETTE[rank % MODEL_PALETTE.length];
}

/** CSS background value for family chips/bars in DOM. */
export function familySwatch(family: string, rank: number): string {
  return cssColor(familyColor(family, rank));
}

/** Flat hex for SVG strokes (Spark lines) where gradients can't go. */
export function familyFlat(family: string, rank: number): string {
  return flatColor(familyColor(family, rank));
}

/**
 * Strip to the last path segment ("anthropic/claude-sonnet-4.5" →
 * "claude-sonnet-4.5"), strip `[suffix]` decorations, trim, and lowercase.
 * Mirror of Rust `pricing::normalize_model`; used for duplicate-model
 * detection and family matching.
 */
export function normalizeModelName(s: string): string {
  const base = s.split('/').pop() ?? s;
  return (base.split('[')[0] ?? base).trim().toLowerCase();
}

/// Mirror of Rust `families::FAMILY_RULES` — prefix order matters (specific
/// prefixes before catch-alls within a brand).
const FAMILY_RULES: ReadonlyArray<readonly [string, string]> = [
  ['o3-mini', 'GPT'], ['o3', 'GPT'], ['o4', 'GPT'], ['codex', 'GPT'], ['gpt', 'GPT'],
  ['claude-opus-4', 'Claude'], ['claude-opus', 'Claude'], ['claude-fable', 'Claude'],
  ['claude-sonnet', 'Claude'], ['claude-haiku', 'Claude'], ['claude', 'Claude'],
  ['gemini-3.7-flash', 'Gemini'], ['gemini-2.5-pro', 'Gemini'], ['gemini', 'Gemini'],
  ['deepseek-v4-pro', 'DeepSeek'], ['deepseek', 'DeepSeek'],
  ['kimi', 'Kimi'], ['qwen', 'Qwen'], ['glm', 'GLM'], ['mimo', 'MiMo'],
  ['meta-llama', 'Meta'], ['llama', 'Meta'], ['muse', 'Meta'], ['codestral', 'Mistral'], ['mistral', 'Mistral'], ['grok', 'Grok'],
];

/// Mirror of Rust `families::family_for` — assigns a model display name to
/// its provider family so per-model UI can wear brand colors.
export function familyFor(model: string): string {
  const base = normalizeModelName(model);
  for (const [prefix, family] of FAMILY_RULES) {
    if (base.startsWith(prefix)) return family;
  }
  return 'Other';
}

/// Brand color for an individual model, resolved through its family.
export function modelColor(model: string, rank: number): ChartColor {
  return familyColor(familyFor(model), rank);
}

/** CSS background value for per-model chips/bars in DOM. */
export function modelSwatch(model: string, rank: number): string {
  return cssColor(modelColor(model, rank));
}

/** Flat hex for per-model SVG strokes (Spark lines). */
export function modelFlat(model: string, rank: number): string {
  return flatColor(modelColor(model, rank));
}
