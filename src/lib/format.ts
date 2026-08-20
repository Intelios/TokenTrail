/// Marathon source palette — must stay in sync with chartTheme.ts tokens.
export const SOURCE_COLORS: Record<string, string> = {
  claude_code: '#ff4d00',
  codex: '#00c2c2',
  zcode: '#7c5cff',
  antigravity: '#c8e600',
  opencode: '#ff1f6f',
  gemini: '#3d8eff',
};

export const SOURCE_LABEL: Record<string, string> = {
  zcode: 'ZCode',
  claude_code: 'Claude Code',
  codex: 'Codex',
  opencode: 'OpenCode',
  gemini: 'Gemini CLI',
  antigravity: 'Antigravity',
};

/// Marathon accents by rank, then ink-shades for the long tail.
export const MODEL_PALETTE = [
  '#ff4d00', '#00c2c2', '#c8e600', '#ff1f6f', '#7c5cff',
  '#3d8eff', '#8a8578', '#6e6a5e', '#4a473e', '#b5afa0', '#2e2c26',
];

/// Input / Output / Cache read / Cache write — shared by charts across pages.
export const MIX_COLORS = ['#3d8eff', '#00c2c2', '#c8e600', '#ff1f6f'];

export function sourceColor(s: string): string {
  return SOURCE_COLORS[s] ?? '#8a8578';
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

/**
 * Mirror of Rust `pricing::normalize_model`: strip to the last path segment
 * ("anthropic/claude-sonnet-4.5" → "claude-sonnet-4.5"), strip `[suffix]`
 * decorations, trim, and lowercase. Used to detect likely-duplicate model
 * names (e.g. "GLM-5.3" vs "glm-5.3") for merge suggestions.
 */
/// Identity palette for model families — stable colors regardless of rank,
/// so "Claude" is always orange and "GPT" always teal.
export const FAMILY_COLORS: Record<string, string> = {
  Claude: '#ff4d00',
  GPT: '#00c2c2',
  Gemini: '#3d8eff',
  DeepSeek: '#ff1f6f',
  Kimi: '#7c5cff',
  Qwen: '#c8e600',
  GLM: '#6e6a5e',
  MiMo: '#b5afa0',
  Other: '#8a8578',
};

export function familyColor(family: string, rank: number): string {
  return FAMILY_COLORS[family] ?? MODEL_PALETTE[rank % MODEL_PALETTE.length];
}

export function normalizeModelName(s: string): string {
  const base = s.split('/').pop() ?? s;
  return (base.split('[')[0] ?? base).trim().toLowerCase();
}
