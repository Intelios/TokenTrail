import { invoke } from '@tauri-apps/api/core';

export interface SourceTotals {
  source: string;
  tokens: number;
  events: number;
  sessions: number;
  cost_usd: number | null;
}

export interface UsagePurposeRow {
  purpose: string;
  requests: number;
  tokens: number;
  cost_usd: number | null;
}

export interface WackCodeOutcomeRow {
  outcome: string;
  events: number;
}

export interface WackCodeBreakdownRow {
  name: string;
  events: number;
  tokens: number;
  cost_usd: number | null;
}

export interface DailyPurposeRow {
  date: string;
  purpose: string;
  tokens: number;
}

export interface WackCodeSessionRow {
  session_id: string;
  project: string;
  /** Work-purpose models only; housekeeping models stay in the call timeline. */
  models: string[];
  events: number;
  subagents: number;
  tokens: number;
  cost_usd: number | null;
  completed: number;
  failed: number;
  cancelled: number;
  first_ts: number;
  last_ts: number;
  /** 'code' or 'chat'; rows written before Chat mode shipped read as 'code'. */
  mode: string;
}

/** One WackCode model call inside a chat, in timeline order. */
export interface WackCodeCallRow {
  ts: number;
  purpose: string;
  subagent_id: string | null;
  model: string;
  outcome: string;
  tokens: number;
  duration_ms: number;
}

export interface WackCodeDetail {
  total_tokens: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  events: number;
  sessions: number;
  active_days: number;
  cost_usd: number | null;
  first_ts: number | null;
  last_ts: number | null;
  subagent_events: number;
  subagent_tokens: number;
  by_purpose: UsagePurposeRow[];
  by_outcome: WackCodeOutcomeRow[];
  /** Tokens/calls split by app mode ('code' covers rows with no mode stamp). */
  by_mode: WackCodeBreakdownRow[];
  by_model: WackCodeBreakdownRow[];
  by_provider: WackCodeBreakdownRow[];
  by_project: WackCodeBreakdownRow[];
  daily: DailyPurposeRow[];
  sessions_list: WackCodeSessionRow[];
}

export interface Overview {
  total_tokens: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  events: number;
  active_days: number;
  /** Total tokens ÷ calendar months with any usage (UTC); idle gaps don't count. */
  avg_monthly_tokens: number;
  months_tracked: number;
  cost_usd: number | null;
  first_ts: number | null;
  last_ts: number | null;
  current_streak: number;
  longest_streak: number;
  by_source: SourceTotals[];
}

export interface DailyRow {
  date: string;
  source: string;
  tokens: number;
  cost_usd: number | null;
}

export interface DailyProjectRow {
  date: string;
  project: string;
  tokens: number;
  cost_usd: number | null;
}

export interface DailyModelRow {
  date: string;
  model: string;
  tokens: number;
}

export interface DailyCacheRow {
  date: string;
  fresh_input: number;
  cache_write: number;
  cache_read: number;
}

export interface ModelRow {
  model: string;
  tokens: number;
  events: number;
  cost_usd: number | null;
  last_ts: number | null;
}

export interface ModelStatsRow {
  model: string;
  tokens: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  events: number;
  sessions: number;
  cost_usd: number | null;
  first_ts: number | null;
  last_ts: number | null;
  sources: string[];
  active_days: number;
}

export interface ProjectRow {
  project: string;
  tokens: number;
  events: number;
  sessions: number;
  cost_usd: number | null;
  first_ts: number | null;
  last_ts: number | null;
}

export interface HeatmapCell {
  date: string;
  tokens: number;
}

export interface HourRow {
  hour: number;
  tokens: number;
}

/**
 * Per-source count of events whose tokens the reporting app estimated rather than had
 * reported to it. Sources with none are omitted, so an empty list is the usual answer.
 */
export interface EstimatedShare {
  source: string;
  events: number;
  estimated: number;
}

export interface SourceStatus {
  source: string;
  display: string;
  path: string;
  found: boolean;
}

export interface IngestStats {
  source: string;
  processed: number;
  error: string | null;
}

/**
 * Events recorded as `alias` in `[from_ts, until_ts)` (epoch ms) display as
 * `canonical`. One raw name can have several segments — a floating API name
 * like `deepseek-flash` split at each release. `until_ts` is `ALIAS_OPEN_END`
 * when the segment has no end.
 */
export interface ModelAlias {
  alias: string;
  canonical: string;
  from_ts: number;
  until_ts: number;
}

/** Rust's `i64::MAX`, rounded by JSON into a float; compare with `>=`. */
export const ALIAS_OPEN_END = 9223372036854775807;

/** A user-pinned chart color; `color` is lowercase `#rrggbb`. */
export interface ProjectColor {
  project: string;
  color: string;
}

/** User-assigned tech stack tags for a project, in display order. */
export interface ProjectStack {
  project: string;
  stack: string[];
}

/**
 * A folder → project mapping: events recorded under `alias` count as `canonical`.
 * Filled automatically from the nearest enclosing git root, and adjustable by hand.
 */
export interface ProjectAlias {
  alias: string;
  canonical: string;
}

export interface FamilyStatsRow {
  family: string;
  tokens: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  events: number;
  sessions: number;
  cost_usd: number | null;
  first_ts: number | null;
  last_ts: number | null;
  sources: string[];
  models: ModelStatsRow[];
}

export interface ModelDetail {
  model: string;
  tokens: number;
  input_tokens: number;
  output_tokens: number;
  reasoning_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  events: number;
  sessions: number;
  cost_usd: number | null;
  first_ts: number | null;
  last_ts: number | null;
  active_days: number;
  current_streak: number;
  longest_streak: number;
  peak_day: string | null;
  peak_day_tokens: number;
  by_source: SourceTotals[];
  by_project: ProjectRow[];
  daily: HeatmapCell[];
  total_window_tokens: number;
}

export interface Achievement {
  kind: string;
  tier: string | null;
  title: string;
  value: string;
  earned_ts: number | null;
}

export interface ProjectModelRow {
  model: string;
  tokens: number;
  input_tokens: number;
  output_tokens: number;
  events: number;
  sessions: number;
  cost_usd: number | null;
  first_ts: number | null;
  last_ts: number | null;
}

export interface SessionRow {
  session_id: string;
  source: string;
  events: number;
  tokens: number;
  cost_usd: number | null;
  first_ts: number;
  last_ts: number;
  models: string[];
}

export interface ProjectDetail {
  project: string;
  tokens: number;
  input_tokens: number;
  output_tokens: number;
  reasoning_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  events: number;
  sessions: number;
  cost_usd: number | null;
  first_ts: number | null;
  last_ts: number | null;
  active_days: number;
  current_streak: number;
  longest_streak: number;
  peak_day: string | null;
  peak_day_tokens: number;
  by_model: ProjectModelRow[];
  by_source: SourceTotals[];
  daily: HeatmapCell[];
  sessions_list: SessionRow[];
  total_window_tokens: number;
}

export interface LeaderboardEvent {
  kind: 'overtake' | 'record' | 'debut' | 'first_seen' | 'promotion' | 'demotion';
  model: string;
  other_model: string | null;
  rank: number | null;
  date: string;
  tokens: number;
  tenure_days: number | null;
}

export interface PeakDayRow {
  model: string;
  date: string;
  tokens: number;
  cost_usd: number;
}

export interface WrappedModelRow {
  model: string;
  tokens: number;
  events: number;
}

export interface WrappedSourceRow {
  source: string;
  tokens: number;
}

/** One trailing window's worth of the shareable "Usage Wrapped" card.
 *  Deliberately carries no cost fields — the card is a token story. */
export interface WrappedSummary {
  period_days: number;
  total_tokens: number;
  events: number;
  sessions: number;
  active_days: number;
  window_days: number;
  current_streak: number;
  longest_streak: number;
  first_ts: number | null;
  last_ts: number | null;
  top_models: WrappedModelRow[];
  by_source: WrappedSourceRow[];
  daily: HeatmapCell[];
  peak_day: string | null;
  peak_day_tokens: number;
  busiest_hour: number | null;
  /** Share of tokens written 22:00–06:00 UTC, 0–1. */
  night_share: number;
  top_project: string | null;
  top_project_tokens: number;
  estimated_tokens: number;
}

export const api = {
  overview: () => invoke<Overview>('get_overview'),
  wackcodeDetail: (days: number) => invoke<WackCodeDetail>('get_wackcode_detail', { days }),
  /** The whole life of one chat, regardless of the page's range filter. */
  wackcodeSessionCalls: (sessionId: string) =>
    invoke<WackCodeCallRow[]>('get_wackcode_session_calls', { sessionId }),
  daily: (days: number) => invoke<DailyRow[]>('get_daily', { days }),
  dailyByModel: (days: number) => invoke<DailyModelRow[]>('get_daily_by_model', { days }),
  dailyCache: (days: number) => invoke<DailyCacheRow[]>('get_daily_cache', { days }),
  byModel: (days: number) => invoke<ModelRow[]>('get_by_model', { days }),
  activeDays: (days: number) => invoke<number>('get_active_days', { days }),
  modelStats: (days: number) => invoke<ModelStatsRow[]>('get_model_stats', { days }),
  modelDetail: (model: string, days: number) =>
    invoke<ModelDetail | null>('get_model_detail', { model, days }),
  modelAchievements: (model: string) =>
    invoke<Achievement[]>('get_model_achievements', { model }),
  byProject: (days: number) => invoke<ProjectRow[]>('get_by_project', { days }),
  dailyByProject: (days: number) => invoke<DailyProjectRow[]>('get_daily_by_project', { days }),
  projectDetail: (project: string, days: number) =>
    invoke<ProjectDetail | null>('get_project_detail', { project, days }),
  heatmap: (days: number) => invoke<HeatmapCell[]>('get_heatmap', { days }),
  hourly: () => invoke<HourRow[]>('get_hourly'),
  sourceStatus: () => invoke<SourceStatus[]>('get_source_status'),

  estimatedShare: () => invoke<EstimatedShare[]>('get_estimated_share'),
  syncNow: () => invoke<IngestStats[]>('sync_now'),
  exportData: (format: 'csv' | 'json') => invoke<string>('export_data', { format }),
  modelAliases: () => invoke<ModelAlias[]>('get_model_aliases'),
  rawModels: () => invoke<string[]>('get_raw_models'),
  mergeModels: (names: string[], canonical: string) =>
    invoke<void>('merge_models', { names, canonical }),
  unmergeModels: (canonical: string) => invoke<void>('unmerge_models', { canonical }),
  renameModel: (currentName: string, newName: string) =>
    invoke<void>('rename_model', { currentName, newName }),
  splitModelAlias: (alias: string, atMs: number, canonical: string) =>
    invoke<void>('split_model_alias', { alias, atMs, canonical }),
  removeModelAliasSegment: (alias: string, fromTs: number) =>
    invoke<void>('remove_model_alias_segment', { alias, fromTs }),
  hiddenModels: () => invoke<string[]>('get_hidden_models'),
  hideModels: (names: string[]) => invoke<void>('hide_models', { names }),
  unhideModel: (name: string) => invoke<void>('unhide_model', { name }),
  removeModelAlias: (alias: string) => invoke<void>('remove_model_alias', { alias }),
  projectColors: () => invoke<ProjectColor[]>('get_project_colors'),
  /** `null` clears the color and hands the project back to the auto palette. */
  setProjectColor: (project: string, color: string | null) =>
    invoke<void>('set_project_color', { project, color }),
  projectStacks: () => invoke<ProjectStack[]>('get_project_stacks'),
  /** An empty `stack` clears all tech stack tags for the project. */
  setProjectStack: (project: string, stack: string[]) =>
    invoke<void>('set_project_stack', { project, stack }),
  projectAliases: () => invoke<ProjectAlias[]>('get_project_aliases'),
  mergeProjects: (names: string[], canonical: string) =>
    invoke<void>('merge_projects', { names, canonical }),
  /** Keep these folders as their own projects, overriding automatic root-folding. */
  unmergeProjects: (names: string[]) => invoke<void>('unmerge_projects', { names }),
  /** Hand these folders back to automatic root-folding (nearest git root). */
  regroupProjects: (names: string[]) => invoke<void>('regroup_projects', { names }),
  familyStats: (days: number) => invoke<FamilyStatsRow[]>('get_family_stats', { days }),
  leaderboardEvents: (days: number) => invoke<LeaderboardEvent[]>('get_leaderboard_events', { days }),
  peakDays: (days: number) => invoke<PeakDayRow[]>('get_peak_days', { days }),
  wrappedSummary: (days: number) => invoke<WrappedSummary>('get_wrapped_summary', { days }),
  /** `path` comes from the save dialog; `dataB64` is the canvas PNG. */
  exportWrappedPng: (path: string, dataB64: string) =>
    invoke<string>('export_wrapped_png', { path, dataB64 }),
};

export const wackchatterApi = {
  libraries: () => invoke<import('./wackchatter/types').Library[]>('get_wackchatter_libraries'),
  stats: (libraryId:string, characterId:string|null=null, groupId:string|null=null) => invoke<import('./wackchatter/types').Stats>('get_wackchatter_stats',{libraryId,characterId,groupId}),
  usage: (libraryId:string|null, days:number, sessionId:string|null=null) => invoke<import('./wackchatter/types').Usage>('get_wackchatter_usage',{libraryId,days,sessionId}),
  portrait: (libraryId:string,id:string,persona=false) => invoke<string>('get_wackchatter_portrait',{libraryId,id,persona}),
  open: (libraryId:string,chatId:string|null=null,characterId:string|null=null) => invoke<void>('open_wackchatter',{libraryId,chatId,characterId}),
};
