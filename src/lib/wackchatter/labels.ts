export const FEATURE_LABELS: Record<string, string> = {
  chat: "Chat",
  summary: "Summaries",
  memory: "Memory",
  cocreator: "Character Co-Creator",
  presetCocreator: "Preset Co-Creator",
  presetTest: "Preset testing",
  arena: "Arena",
  persona: "Persona derivation",
  impersonate: "Impersonation",
  recap: "Recaps",
  groupDirector: "Group director",
  groupProfiles: "Group profiles",
  unknown: "Unknown",
};

export const FEATURE_COLORS: Record<string, string> = {
  chat: "#00c2c2",
  summary: "#c8e600",
  memory: "#ff1f6f",
  recap: "#3d8eff",
  arena: "#ff4d00",
  groupDirector: "#0d0d0b",
  groupProfiles: "#8a8578",
  persona: "#e0a030",
  impersonate: "#2e9e6b",
  cocreator: "#c45fb0",
  presetCocreator: "#a0708a",
  presetTest: "#607080",
  unknown: "#b8b2a4",
};

export const featureLabel = (k: string) => FEATURE_LABELS[k] ?? k;
export const featureColor = (k: string) => FEATURE_COLORS[k] ?? "#b8b2a4";

/** Estimated counts get their own colour everywhere, never a measured one. */
export const ESTIMATED = "#7c5cff";

export const OUTCOME_COLORS: Record<string, string> = {
  completed: "#0d0d0b",
  cancelled: "#ff1f6f",
  failed: "#ff4d00",
  unknown: "#b8b2a4",
};

export const count = (n: number) => Math.round(n).toLocaleString();
export const date = (n: number | null) =>
  n === null
    ? "—"
    : new Date(n).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
export const duration = (n: number | null) =>
  n === null ? "—" : n < 1000 ? `${Math.round(n)}ms` : `${(n / 1000).toFixed(1)}s`;
export const percent = (n: number, d: number) => (d ? `${((n / d) * 100).toFixed(1)}%` : "—");
