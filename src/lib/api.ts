import { invoke } from '@tauri-apps/api/core';

export interface SourceTotals {
  source: string;
  tokens: number;
  events: number;
  sessions: number;
  cost_usd: number | null;
}

export interface Overview {
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

export interface ModelAlias {
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
}

export const api = {
  overview: () => invoke<Overview>('get_overview'),
  daily: (days: number) => invoke<DailyRow[]>('get_daily', { days }),
  dailyByModel: (days: number) => invoke<DailyModelRow[]>('get_daily_by_model', { days }),
  dailyCache: (days: number) => invoke<DailyCacheRow[]>('get_daily_cache', { days }),
  byModel: (days: number) => invoke<ModelRow[]>('get_by_model', { days }),
  modelStats: (days: number) => invoke<ModelStatsRow[]>('get_model_stats', { days }),
  modelDetail: (model: string) => invoke<ModelDetail | null>('get_model_detail', { model }),
  byProject: (days: number) => invoke<ProjectRow[]>('get_by_project', { days }),
  heatmap: (days: number) => invoke<HeatmapCell[]>('get_heatmap', { days }),
  hourly: () => invoke<HourRow[]>('get_hourly'),
  sourceStatus: () => invoke<SourceStatus[]>('get_source_status'),
  syncNow: () => invoke<IngestStats[]>('sync_now'),
  exportData: (format: 'csv' | 'json') => invoke<string>('export_data', { format }),
  modelAliases: () => invoke<ModelAlias[]>('get_model_aliases'),
  mergeModels: (names: string[], canonical: string) =>
    invoke<void>('merge_models', { names, canonical }),
  unmergeModels: (canonical: string) => invoke<void>('unmerge_models', { canonical }),
  renameModel: (currentName: string, newName: string) =>
    invoke<void>('rename_model', { currentName, newName }),
  hiddenModels: () => invoke<string[]>('get_hidden_models'),
  hideModels: (names: string[]) => invoke<void>('hide_models', { names }),
  unhideModel: (name: string) => invoke<void>('unhide_model', { name }),
  removeModelAlias: (alias: string) => invoke<void>('remove_model_alias', { alias }),
  familyStats: (days: number) => invoke<FamilyStatsRow[]>('get_family_stats', { days }),
};
