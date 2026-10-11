export interface Library {
  id: string;
  name: string;
  available: boolean;
  current: boolean;
}
export interface Totals {
  chats: number;
  messages: number;
  user_messages: number;
  replies: number;
  swipes: number;
  measured_tokens: number;
  estimated_tokens: number;
  active_minutes: number;
}
export interface Habits {
  rerolls: number;
  rerolled_replies: number;
  eligible_replies: number;
  avg_user_chars: number;
  avg_reply_chars: number;
  longest_reply_chars: number;
  reasoning_swipes: number;
  timed_swipes: number;
  branched_chats: number;
  deleted_chats: number;
}
export interface Chat {
  id: string;
  title: string;
  kind: string;
  character_id: string | null;
  messages: number;
  replies: number;
  measured_tokens: number;
  estimated_tokens: number;
  created: number;
  modified: number;
}
export interface Cast {
  character_id: string;
  name: string;
  chats: number;
  replies: number;
  measured_tokens: number;
  estimated_tokens: number;
  first_chat: number;
  last_chat: number;
}
export interface Model {
  model: string;
  provider: string;
  swipes: number;
  measured_tokens: number;
  estimated_tokens: number;
  timed_swipes: number;
  total_ms: number;
  measured_ms: number;
  measured_timed_tokens: number;
  estimated_ms: number;
  estimated_timed_tokens: number;
}
export interface Persona {
  persona_id: string | null;
  name: string;
  chats: number;
  messages: number;
}
export interface Stats {
  totals: Totals;
  habits: Habits;
  cast: Cast[];
  chats: Chat[];
  groups: Chat[];
  models: Model[];
  personas: Persona[];
  hours: [number, number][];
  co_creator_sessions: number;
  first_chat: number | null;
  last_chat: number | null;
  name: string;
}
export interface UsageTotal {
  events: number;
  measured_events: number;
  priced_events: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  estimated_tokens: number;
  cost_usd: number | null;
  avg_duration_ms: number | null;
  avg_ttft_ms: number | null;
}
export interface Breakdown {
  name: string;
  events: number;
  measured_tokens: number;
  estimated_tokens: number;
  cost_usd: number | null;
}
export interface Call {
  id: string;
  ts: number;
  model: string;
  purpose: string;
  outcome: string;
  measured_tokens: number;
  estimated_tokens: number;
  cost_usd: number | null;
  duration_ms: number | null;
  ttft_ms: number | null;
}
export interface Usage {
  totals: UsageTotal;
  features: Breakdown[];
  models: Breakdown[];
  providers: Breakdown[];
  characters: Breakdown[];
  conversations: Breakdown[];
  outcomes: Breakdown[];
  daily: Breakdown[];
  calls: Call[];
}
