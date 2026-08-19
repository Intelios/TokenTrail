export const SOURCE_COLORS: Record<string, string> = {
  zcode: '#a78bfa',
  claude_code: '#fb923c',
  codex: '#34d399',
  opencode: '#f472b6',
  gemini: '#60a5fa',
  antigravity: '#22d3ee',
};

export const SOURCE_LABEL: Record<string, string> = {
  zcode: 'ZCode',
  claude_code: 'Claude Code',
  codex: 'Codex',
  opencode: 'OpenCode',
  gemini: 'Gemini CLI',
  antigravity: 'Antigravity',
};

export const MODEL_PALETTE = [
  '#a78bfa', '#fb923c', '#34d399', '#60a5fa', '#f472b6',
  '#facc15', '#22d3ee', '#f87171', '#c084fc', '#4ade80', '#94a3b8',
];

/// Input / Output / Cache read / Cache write — shared by charts across pages.
export const MIX_COLORS = ['#60a5fa', '#34d399', '#facc15', '#f472b6'];

export function sourceColor(s: string): string {
  return SOURCE_COLORS[s] ?? '#94a3b8';
}

export function sourceLabel(s: string): string {
  return SOURCE_LABEL[s] ?? s;
}

export function fmtTokens(n: number): string {
  if (!isFinite(n)) return '—';
  if (n >= 1e9) return (n / 1e9).toFixed(n >= 1e10 ? 0 : 1) + 'B';
  if (n >= 1e6) return (n / 1e6).toFixed(n >= 1e7 ? 0 : 1) + 'M';
  if (n >= 1e3) return (n / 1e3).toFixed(n >= 1e4 ? 0 : 1) + 'K';
  return String(n);
}

export function fmtCost(n: number | null): string {
  if (n == null) return '—';
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
 * Mirror of Rust `pricing::normalize_model`: strip `provider/` prefixes and
 * `[suffix]` decorations, trim, and lowercase. Used to detect likely-duplicate
 * model names (e.g. "GLM-5.3" vs "glm-5.3") for merge suggestions.
 */
export function normalizeModelName(s: string): string {
  const base = s.split('/')[0];
  return (base.split('[')[0] ?? base).trim().toLowerCase();
}
