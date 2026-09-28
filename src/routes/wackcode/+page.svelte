<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import AnimatedNumber from '$lib/AnimatedNumber.svelte';
  import Chart from '$lib/Chart.svelte';
  import Spark from '$lib/Spark.svelte';
  import { api, type DailyRow, type ProjectColor, type WackCodeCallRow, type WackCodeDetail } from '$lib/api';
  import {
    MODEL_PALETTE,
    basename,
    fmtCost,
    fmtDate,
    fmtDuration,
    fmtTokens,
    fmtTokensExact,
    fmtTokensSplit,
    markedColors,
    modelSwatch,
    resolveProjectColors,
  } from '$lib/format';
  import { dailyColumns, dailyOption, dailyRange, spineTotals } from '$lib/dailyColumns';
  import { readPref, writePref } from '$lib/prefs';

  const RANGES: [number, string][] = [
    [7, '7D'],
    [30, '30D'],
    [90, '90D'],
    [3650, 'ALL'],
  ];
  const RANGE_LABEL: Record<number, string> = { 7: '7 days', 30: '30 days', 90: '90 days', 3650: 'all time' };
  const PREF_DAYS = 'tt.wackcode.days';

  // stack order for the daily chart; unknown purposes ride on top
  const PURPOSE_ORDER = [
    'chat',
    'subagent',
    'compaction',
    'title',
    'branch_summary',
    'goal_verification',
    'commit_message',
  ];
  const PURPOSE_LABELS: Record<string, string> = {
    chat: 'Chat',
    subagent: 'Sub-agents',
    title: 'Titles',
    compaction: 'Compaction',
    branch_summary: 'Branch summaries',
    goal_verification: 'Goal verification',
    commit_message: 'Commit messages',
  };
  const PURPOSE_TAGS: Record<string, string> = {
    chat: 'CHAT',
    subagent: 'SUB',
    title: 'TITLE',
    compaction: 'COMPACT',
    branch_summary: 'BRANCH',
    goal_verification: 'VERIFY',
    commit_message: 'COMMIT',
  };
  const PURPOSE_COLORS: Record<string, string> = {
    chat: '#00c2c2', // cyn — the WackCode signature
    subagent: '#3d8eff',
    compaction: '#7c5cff',
    title: '#c8e600',
    branch_summary: '#ff1f6f',
    goal_verification: '#ff4d00',
    commit_message: '#8a8578',
  };
  const OUTCOME_COLORS: Record<string, string> = {
    completed: '#00c2c2',
    failed: '#ff1f6f',
    cancelled: 'rgba(13,13,11,0.35)',
  };
  const OUTCOME_ORDER = ['completed', 'failed', 'cancelled'];
  const HOUSEKEEPING = ['title', 'compaction', 'branch_summary', 'goal_verification', 'commit_message'];

  let days = $state(readPref(PREF_DAYS, 90, (v) => RANGES.some(([d]) => d === v)));
  let detail = $state<WackCodeDetail | null>(null);
  let projectColors = $state<ProjectColor[]>([]);
  let error = $state('');
  let showAllSessions = $state(false);
  let copiedId = $state('');
  // expansion state is per chat row; call lists load once and stay cached
  let expanded = $state<Record<string, boolean>>({});
  let callsBySession = $state<Record<string, WackCodeCallRow[]>>({});
  let loadingCalls = $state<Record<string, boolean>>({});

  async function load() {
    try {
      const [d, pc] = await Promise.all([api.wackcodeDetail(days), api.projectColors()]);
      detail = d;
      projectColors = pc;
      error = '';
    } catch (e) {
      error = String(e);
    }
  }

  $effect(() => {
    days;
    load();
  });

  $effect(() => {
    writePref(PREF_DAYS, days);
  });

  onMount(() => {
    load();
    const h = () => load();
    window.addEventListener('tt-sync', h);
    return () => window.removeEventListener('tt-sync', h);
  });

  async function toggleSession(sessionId: string) {
    expanded[sessionId] = !expanded[sessionId];
    if (expanded[sessionId] && !callsBySession[sessionId] && !loadingCalls[sessionId]) {
      loadingCalls[sessionId] = true;
      try {
        callsBySession[sessionId] = await api.wackcodeSessionCalls(sessionId);
      } catch (e) {
        error = String(e);
        expanded[sessionId] = false;
      } finally {
        loadingCalls[sessionId] = false;
      }
    }
  }

  async function copySession(id: string) {
    try {
      await navigator.clipboard.writeText(id);
      copiedId = id;
      setTimeout(() => {
        if (copiedId === id) copiedId = '';
      }, 1200);
    } catch {
      /* clipboard is best-effort */
    }
  }

  // ── daily stacked columns, keyed by purpose instead of source ──
  const purposes = $derived.by(() => {
    if (!detail) return [];
    const seen = new Set<string>([
      ...detail.daily.map((r) => r.purpose),
      ...detail.by_purpose.map((p) => p.purpose),
    ]);
    return PURPOSE_ORDER.filter((p) => seen.has(p)).concat([...seen].filter((p) => !PURPOSE_ORDER.includes(p)));
  });

  const dailyAsRows = $derived(
    (detail?.daily ?? []).map((r): DailyRow => ({ date: r.date, source: r.purpose, tokens: r.tokens, cost_usd: null })),
  );
  const dayCols = $derived(dailyColumns(dailyAsRows, purposes));
  const dayOpt = $derived(
    dailyOption(dayCols, purposes, {
      label: (p) => PURPOSE_LABELS[p] ?? p,
      color: (p) => PURPOSE_COLORS[p] ?? '#8a8578',
    }),
  );
  const dayRng = $derived(dailyRange(dayCols));
  const dailyTotals = $derived(spineTotals(dayCols));

  // ── quads ──
  const completed = $derived(detail?.by_outcome.find((o) => o.outcome === 'completed')?.events ?? 0);
  const failedCount = $derived(detail?.by_outcome.find((o) => o.outcome === 'failed')?.events ?? 0);
  const cancelledCount = $derived(detail?.by_outcome.find((o) => o.outcome === 'cancelled')?.events ?? 0);
  const successPct = $derived(detail && detail.events ? (completed / detail.events) * 100 : 0);
  const subShare = $derived(detail && detail.total_tokens ? (detail.subagent_tokens / detail.total_tokens) * 100 : 0);

  const outcomeRows = $derived.by(() => {
    if (!detail) return [];
    const counts: Record<string, number> = Object.fromEntries(detail.by_outcome.map((o) => [o.outcome, o.events]));
    return OUTCOME_ORDER.map((o) => ({ outcome: o, events: counts[o] ?? 0 })).filter((r) => r.events > 0);
  });

  // ── rank bars ──
  const maxPurposeTokens = $derived(Math.max(1, ...(detail?.by_purpose ?? []).map((p) => p.tokens)));
  const maxModelTokens = $derived(Math.max(1, ...(detail?.by_model ?? []).slice(0, 6).map((m) => m.tokens)));

  const projectColorMap = $derived(
    resolveProjectColors((detail?.by_project ?? []).map((p) => p.name), markedColors(projectColors)),
  );

  const displayedSessions = $derived(
    showAllSessions ? (detail?.sessions_list ?? []) : (detail?.sessions_list ?? []).slice(0, 50),
  );

  const rangeLabel = $derived(RANGE_LABEL[days] ?? '');

  const modelUrl = (m: string) => '/models/' + encodeURIComponent(m);
  const projectUrl = (p: string) => '/projects/' + encodeURIComponent(p);
  const callTime = (ts: number) => new Date(ts).toLocaleTimeString(undefined, { hour12: false });

  // ── NOTE: the most story-worthy fact about this window ──
  const note = $derived.by(() => {
    const d = detail;
    if (!d || !d.events || !d.total_tokens) return null;
    const subPct = Math.round(subShare);
    const hkCalls = d.by_purpose.filter((p) => HOUSEKEEPING.includes(p.purpose)).reduce((a, b) => a + b.requests, 0);
    const hkTokens = d.by_purpose.filter((p) => HOUSEKEEPING.includes(p.purpose)).reduce((a, b) => a + b.tokens, 0);
    const hkCallPct = Math.round((hkCalls / d.events) * 100);
    const hkTokenPct = Math.round((hkTokens / d.total_tokens) * 100);
    const notGreat = failedCount + cancelledCount;
    if (subPct >= 15) {
      return `Sub-agents carry real weight — <b>${subPct}%</b> of WackCode tokens (${fmtTokens(d.subagent_tokens)}) ran inside child calls, <b>${d.subagent_events.toLocaleString()}</b> of them.`;
    }
    if (hkCallPct >= 20 && hkTokenPct < hkCallPct) {
      return `Housekeeping is loud but cheap — titles, compaction and summaries make <b>${hkCallPct}%</b> of calls yet only <b>${hkTokenPct}%</b> of tokens.`;
    }
    if (notGreat > 0) {
      return `<b>${notGreat.toLocaleString()}</b> call${notGreat === 1 ? '' : 's'} didn't finish — ${failedCount.toLocaleString()} failed, ${cancelledCount.toLocaleString()} cancelled.`;
    }
    return `A clean window — <b>${d.events.toLocaleString()}</b> calls, every one of them completed.`;
  });
</script>

<div class="wframe">
  {#if error}
    <p class="error">{error}</p>
  {:else if !detail}
    <div class="loading">loading WackCode…</div>
  {:else if !detail.events}
    <div class="empty">
      <div class="e-kick up">WACKCODE</div>
      <div class="e-hero up" style="animation-delay:80ms">Nothing recorded yet</div>
      <p class="up" style="animation-delay:160ms">
        WackCode writes a usage ledger to
        <code>~/Library/Application Support/com.wackcode.desktop/usage/v1</code>. If the app is running, make sure
        recording is on in WackCode Settings → Integrations, then hit Sync.
      </p>
    </div>
  {:else}
    <!-- header -->
    <section class="thd">
      <div class="tt up">
        <h1>WackCode</h1>
        <div class="sub">
          {detail.sessions.toLocaleString()} chats · {detail.events.toLocaleString()} calls · {rangeLabel}
        </div>
      </div>
      <div class="pills up" style="animation-delay:80ms">
        {#each RANGES as [d, label]}
          <button class="pill" class:on={days === d} onclick={() => (days = d)}>{label}</button>
        {/each}
      </div>
    </section>

    <!-- hero band -->
    <section class="band">
      <div class="hero reg up">
        <div class="kick"><span>WackCode tokens · {rangeLabel}</span></div>
        <div class="n">
          <AnimatedNumber value={detail.total_tokens} format={(n) => fmtTokensSplit(n).value} />
          {#if fmtTokensSplit(detail.total_tokens).unit}
            <u>{fmtTokensSplit(detail.total_tokens).unit}</u>
          {/if}
        </div>
        <div class="sp">
          {#if dailyTotals.length > 1}
            <Spark values={dailyTotals} width={460} height={30} color="var(--cyn)" delay={350} />
          {/if}
        </div>
      </div>
      <div class="quad">
        <div class="q org up" style="animation-delay:70ms">
          <div class="k">Est. cost</div>
          <div class="v"><AnimatedNumber value={detail.cost_usd ?? 0} format={fmtCost} /></div>
          <div class="h">api-equivalent estimate</div>
        </div>
        <div class="q up" style="animation-delay:140ms">
          <div class="k">Chats</div>
          <div class="v"><AnimatedNumber value={detail.sessions} /></div>
          <div class="h">{detail.events.toLocaleString()} model calls</div>
        </div>
        <div class="q cyn up" style="animation-delay:210ms">
          <div class="k">Success</div>
          <div class="v"><AnimatedNumber value={successPct} format={(n) => `${Math.round(n)}%`} /></div>
          <div class="h">
            {failedCount.toLocaleString()} failed · {cancelledCount.toLocaleString()} cancelled
          </div>
        </div>
        <div class="q acd up" style="animation-delay:280ms">
          <div class="k">Sub-agents</div>
          <div class="v"><AnimatedNumber value={subShare} format={(n) => `${Math.round(n)}%`} /></div>
          <div class="h">{fmtTokens(detail.subagent_tokens)} in {detail.subagent_events.toLocaleString()} child calls</div>
        </div>
      </div>
    </section>

    <!-- daily stacked columns by purpose -->
    <section class="daily">
      <div class="hd">
        <h3>Daily tokens by purpose{dayRng ? ` — ${dayRng}` : ''}</h3>
        <div class="rt">
          {#if detail.active_days}
            <b>{detail.active_days.toLocaleString()} ACTIVE {detail.active_days === 1 ? 'DAY' : 'DAYS'}</b>
          {/if}
        </div>
      </div>
      <div class="legend">
        {#each purposes as p}
          <span><i style="background:{PURPOSE_COLORS[p] ?? '#8a8578'}"></i>{PURPOSE_LABELS[p] ?? p}</span>
        {/each}
      </div>
      <div class="plot">
        {#if dayOpt}
          <Chart option={dayOpt} height="fill" />
        {:else}
          <div class="loading">no usage recorded yet</div>
        {/if}
      </div>
    </section>

    <!-- work mix + reliability -->
    <section class="mid">
      <div class="bars">
        <h3><span>Work mix</span><span class="cnt">{detail.by_purpose.length} purposes</span></h3>
        {#each detail.by_purpose as p, i}
          <div class="rankbar up" style="animation-delay:{120 + i * 60}ms">
            <span class="chip" style="background:{PURPOSE_COLORS[p.purpose] ?? '#8a8578'}">{i + 1}</span>
            <span class="nm" title={p.purpose}>{PURPOSE_LABELS[p.purpose] ?? p.purpose}</span>
            <span class="tr" data-tip="{PURPOSE_LABELS[p.purpose] ?? p.purpose} · {fmtTokensExact(p.tokens)} tokens · {p.requests.toLocaleString()} calls">
              <div
                class="gw"
                style="width:{Math.max(2, Math.round((p.tokens / maxPurposeTokens) * 100))}%;background:{PURPOSE_COLORS[p.purpose] ?? '#8a8578'};animation-delay:{180 + i * 60}ms"
              ></div>
            </span>
            <b><AnimatedNumber value={p.tokens} format={fmtTokens} duration={1100} /></b>
            <span class="pct">{p.requests.toLocaleString()} calls</span>
          </div>
        {/each}
      </div>
      <div class="rel">
        <h3><span>Reliability</span></h3>
        <div class="obar up" style="animation-delay:160ms">
          {#each outcomeRows as o, i}
            <div
              class="gw seg"
              style="width:{(o.events / detail.events) * 100}%;background:{OUTCOME_COLORS[o.outcome]};animation-delay:{220 + i * 60}ms"
            ></div>
          {/each}
        </div>
        <div class="oleg">
          {#each outcomeRows as o, i}
            <div class="row up" style="animation-delay:{240 + i * 60}ms">
              <i style="background:{OUTCOME_COLORS[o.outcome]}"></i>
              <span class="onm">{o.outcome}</span>
              <span class="ov"><AnimatedNumber value={o.events} duration={1000} /></span>
              <span class="op">{Math.round((o.events / detail.events) * 100)}%</span>
            </div>
          {/each}
        </div>
        <div class="lat">
          <div class="cell up" style="animation-delay:360ms">
            <div class="k">Avg call</div>
            <div class="v"><AnimatedNumber value={detail.avg_duration_ms} format={fmtDuration} /></div>
          </div>
          <div class="cell up" style="animation-delay:430ms">
            <div class="k">Median call</div>
            <div class="v"><AnimatedNumber value={detail.p50_duration_ms} format={fmtDuration} /></div>
          </div>
        </div>
      </div>
    </section>

    <!-- models + providers/projects -->
    <section class="mid">
      <div class="bars">
        <h3><span>Models</span><span class="cnt">{detail.by_model.length} used</span></h3>
        {#each detail.by_model.slice(0, 6) as m, i}
          <div
            class="rankbar up clickable"
            style="animation-delay:{120 + i * 60}ms"
            role="link"
            tabindex="0"
            onclick={() => goto(modelUrl(m.name))}
            onkeydown={(e) => e.key === 'Enter' && goto(modelUrl(m.name))}
            title={m.name}
          >
            <span class="chip" style="background:{modelSwatch(m.name, i)}">{i + 1}</span>
            <span class="nm" title={m.name}>{m.name}</span>
            <span class="tr" data-tip="{m.name} · {fmtTokensExact(m.tokens)} tokens · {m.events.toLocaleString()} calls">
              <div
                class="gw"
                style="width:{Math.max(2, Math.round((m.tokens / maxModelTokens) * 100))}%;background:{modelSwatch(m.name, i)};animation-delay:{180 + i * 60}ms"
              ></div>
            </span>
            <b><AnimatedNumber value={m.tokens} format={fmtTokens} duration={1100} /></b>
            <span class="pct">{m.events.toLocaleString()} calls</span>
          </div>
        {/each}
      </div>
      <div class="side">
        <div class="mini">
          <h3><span>Providers</span></h3>
          {#each detail.by_provider.slice(0, 4) as pr, i}
            <div class="mrow up" style="animation-delay:{160 + i * 60}ms">
              <i style="background:{MODEL_PALETTE[i % MODEL_PALETTE.length]}"></i>
              <span class="nm" title={pr.name}>{pr.name}</span>
              <b><AnimatedNumber value={pr.tokens} format={fmtTokens} duration={1000} /></b>
              <span class="pct">{Math.round((pr.tokens / detail.total_tokens) * 100)}%</span>
            </div>
          {/each}
        </div>
        <div class="mini">
          <h3><span>Projects</span></h3>
          {#each detail.by_project.slice(0, 4) as pj, i}
            <div
              class="mrow up clickable"
              style="animation-delay:{240 + i * 60}ms"
              role="link"
              tabindex="0"
              onclick={() => goto(projectUrl(pj.name))}
              onkeydown={(e) => e.key === 'Enter' && goto(projectUrl(pj.name))}
              title={pj.name}
            >
              <i style="background:{projectColorMap.get(pj.name) ?? '#8a8578'}"></i>
              <span class="nm" title={pj.name}>{pj.name === 'unknown' ? 'unknown' : basename(pj.name)}</span>
              <b><AnimatedNumber value={pj.tokens} format={fmtTokens} duration={1000} /></b>
              <span class="pct">{Math.round((pj.tokens / detail.total_tokens) * 100)}%</span>
            </div>
          {/each}
        </div>
      </div>
    </section>

    <!-- chats -->
    <section class="chats">
      <div class="sec-hd">
        <h2>Chats ({detail.sessions_list.length.toLocaleString()})</h2>
        {#if detail.sessions_list.length > 50}
          <span class="sec-note">Showing {displayedSessions.length} of {detail.sessions_list.length.toLocaleString()} · click a row for its call timeline</span>
        {:else}
          <span class="sec-note">Click a row for its call timeline</span>
        {/if}
      </div>
      <div class="tw">
        <table>
          <thead>
            <tr>
              <th>Chat</th>
              <th>Project</th>
              <th>Models</th>
              <th class="num">Calls</th>
              <th class="num">Sub-agents</th>
              <th class="num">Tokens</th>
              <th class="num">Est. cost</th>
              <th class="num">Success</th>
              <th class="num">Started</th>
              <th class="num">Span</th>
            </tr>
          </thead>
          <tbody>
            {#each displayedSessions as s, i}
              <tr
                class="up srow"
                class:open={expanded[s.session_id]}
                style="animation-delay:{Math.min(i, 20) * 30}ms"
                onclick={() => toggleSession(s.session_id)}
              >
                <td>
                  <div class="sid-wrap">
                    <span class="tri" class:open={expanded[s.session_id]}>▸</span>
                    <span class="sid" title={s.session_id}>{s.session_id.slice(0, 8)}</span>
                    <button
                      class="copy-btn"
                      onclick={(e) => {
                        e.stopPropagation();
                        copySession(s.session_id);
                      }}
                      title="Copy full chat id"
                    >
                      {copiedId === s.session_id ? '✓' : '⧉'}
                    </button>
                  </div>
                </td>
                <td>
                  {#if s.project === 'unknown'}
                    <span class="muted">unknown</span>
                  {:else}
                    <a href={projectUrl(s.project)} onclick={(e) => e.stopPropagation()} title={s.project}>
                      {basename(s.project)}
                    </a>
                  {/if}
                </td>
                <td>
                  <div class="mchips">
                    {#each s.models.slice(0, 3) as m}
                      <span class="modtag">{m}</span>
                    {/each}
                    {#if s.models.length > 3}
                      <span class="modtag more">+{s.models.length - 3}</span>
                    {/if}
                  </div>
                </td>
                <td class="num">{s.events.toLocaleString()}</td>
                <td class="num">{s.subagents ? s.subagents.toLocaleString() : '—'}</td>
                <td class="num">{fmtTokens(s.tokens)}</td>
                <td class="num">{fmtCost(s.cost_usd)}</td>
                <td class="num">
                  {#if s.failed || s.cancelled}
                    <span class="warn" title="{s.failed} failed · {s.cancelled} cancelled">
                      {Math.round((s.completed / s.events) * 100)}%
                    </span>
                  {:else}
                    100%
                  {/if}
                </td>
                <td class="num muted">{fmtDate(s.first_ts)}</td>
                <td class="num muted">{fmtDuration(s.last_ts - s.first_ts)}</td>
              </tr>
              {#if expanded[s.session_id]}
                <tr class="xrow">
                  <td colspan="10">
                    <div class="tlwrap">
                      <div class="tl">
                        {#if loadingCalls[s.session_id]}
                          <div class="loading">loading calls…</div>
                        {:else if (callsBySession[s.session_id] ?? []).length === 0}
                          <div class="loading">no calls visible — a hidden model may be involved</div>
                        {:else}
                          {#each callsBySession[s.session_id] ?? [] as c}
                            <div class="call" class:bad={c.outcome !== 'completed'}>
                              <span class="t">{callTime(c.ts)}</span>
                              <span class="ptag" style="background:{PURPOSE_COLORS[c.purpose] ?? '#8a8578'}">
                                {PURPOSE_TAGS[c.purpose] ?? c.purpose.toUpperCase()}
                              </span>
                              <span class="who" class:sub={c.subagent_id}>
                                {c.subagent_id ? `└ ${c.subagent_id.slice(0, 8)}` : ''}
                              </span>
                              <span class="m" title={c.model}>{c.model}</span>
                              <span class="tok">{fmtTokens(c.tokens)}</span>
                              <span class="dur">{fmtDuration(c.duration_ms)}</span>
                              {#if c.outcome !== 'completed'}
                                <span class="out">{c.outcome}</span>
                              {/if}
                            </div>
                          {/each}
                        {/if}
                      </div>
                    </div>
                  </td>
                </tr>
              {/if}
            {/each}
          </tbody>
        </table>
      </div>
      {#if detail.sessions_list.length > 50}
        <div class="expand-bar">
          <button class="pill on" onclick={() => (showAllSessions = !showAllSessions)}>
            {showAllSessions ? 'Show less (first 50)' : `Show all ${detail.sessions_list.length.toLocaleString()} chats`}
          </button>
        </div>
      {/if}
    </section>

    <!-- note band -->
    {#if note}
      <div class="noteband up" style="animation-delay:420ms">
        <span class="fg">NOTE</span>
        <p>{@html note}</p>
      </div>
    {/if}
  {/if}
</div>

<style>
  .wframe {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow-y: auto;
  }

  /* ── header ── */
  .thd {
    display: flex;
    justify-content: space-between;
    align-items: flex-end;
    gap: 16px;
    padding: clamp(16px, 1.8vh, 26px) clamp(22px, 1.8vw, 40px);
    border-bottom: 2px solid var(--ink);
  }
  .thd h1 {
    margin: 0;
    font: 400 clamp(30px, 2.6vw, 46px) / 0.95 var(--font-disp);
    letter-spacing: -1px;
    text-transform: uppercase;
  }
  .thd .sub {
    margin-top: 7px;
    font: 500 clamp(10px, 0.75vw, 13px) / 1 var(--font-mono);
    letter-spacing: 1px;
    text-transform: uppercase;
    opacity: 0.6;
  }

  /* ── hero band ── */
  .band {
    display: grid;
    grid-template-columns: 1.06fr 0.94fr;
    border-bottom: 2px solid var(--ink);
  }
  .hero {
    padding: clamp(16px, 1.6vh, 26px) clamp(22px, 1.8vw, 40px) clamp(14px, 1.4vh, 24px);
    border-right: 2px solid var(--ink);
    position: relative;
    overflow: hidden;
  }
  .hero .kick {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    font-family: var(--font-ui);
    font-weight: 600;
    font-size: clamp(10px, 0.75vw, 13px);
    line-height: 1;
    letter-spacing: 1.7px;
    text-transform: uppercase;
  }
  .hero .n {
    font-family: var(--font-disp);
    font-weight: 400;
    font-size: clamp(72px, 6.2vw, 170px);
    line-height: 0.8;
    letter-spacing: -2px;
    margin: clamp(15px, 2vh, 30px) 0 0;
    display: flex;
    align-items: flex-start;
    font-variant-numeric: tabular-nums;
  }
  .hero .n u {
    text-decoration: none;
    font-size: clamp(28px, 2.4vw, 64px);
    color: var(--cyn);
    margin-left: clamp(4px, 0.4vw, 10px);
    margin-top: clamp(6px, 0.6vw, 16px);
  }
  .hero .sp {
    margin-top: clamp(12px, 1.4vh, 22px);
    height: clamp(30px, 2.4vw, 56px);
  }
  .hero .sp :global(svg) {
    width: 100%;
    height: 100%;
    display: block;
  }
  .quad {
    display: grid;
    grid-template-columns: 1fr 1fr;
    grid-template-rows: 1fr 1fr;
  }
  .q {
    padding: clamp(13px, 1.5vh, 24px) clamp(17px, 1.4vw, 28px);
    border-right: 2px solid var(--ink);
    border-bottom: 2px solid var(--ink);
  }
  .q:nth-child(2n) {
    border-right: none;
  }
  .q:nth-child(n + 3) {
    border-bottom: none;
  }
  .q .k {
    font: 600 clamp(9px, 0.7vw, 12px) / 1 var(--font-ui);
    letter-spacing: 1.4px;
    text-transform: uppercase;
    opacity: 0.6;
  }
  .q .v {
    font: 400 clamp(32px, 2.7vw, 58px) / 1 var(--font-disp);
    margin-top: clamp(8px, 1vh, 16px);
    letter-spacing: -1px;
    font-variant-numeric: tabular-nums;
  }
  .q .h {
    font: 400 clamp(10px, 0.75vw, 13px) / 1.3 var(--font-mono);
    margin-top: clamp(7px, 0.8vh, 13px);
    opacity: 0.52;
  }
  .q.org {
    background: var(--org);
    color: #fff;
  }
  .q.org .k,
  .q.org .h {
    opacity: 0.82;
  }
  .q.cyn {
    background: var(--cyn);
  }
  .q.acd {
    background: var(--acd);
  }

  /* ── daily chart ── */
  .daily {
    display: flex;
    flex-direction: column;
    padding: clamp(13px, 1.3vh, 22px) clamp(22px, 1.8vw, 40px) clamp(10px, 1vh, 16px);
    border-bottom: 2px solid var(--ink);
    flex: 1 1 auto;
    min-height: 250px;
  }
  .daily .hd {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    flex: none;
  }
  .daily .hd h3 {
    font: 600 clamp(11px, 0.85vw, 15px) / 1 var(--font-ui);
    letter-spacing: 1.6px;
    text-transform: uppercase;
    margin: 0;
  }
  .daily .hd .rt b {
    font: 500 clamp(10px, 0.75vw, 13px) / 1 var(--font-mono);
    letter-spacing: 1px;
    color: var(--cyn);
  }
  .daily .legend {
    display: flex;
    gap: 16px;
    margin-top: 10px;
    flex: none;
    flex-wrap: wrap;
  }
  .daily .legend span {
    display: flex;
    align-items: center;
    gap: 5px;
    font: 500 10px / 1 var(--font-mono);
    letter-spacing: 0.8px;
    text-transform: uppercase;
    opacity: 0.7;
  }
  .daily .legend i {
    width: 10px;
    height: 10px;
    display: inline-block;
    flex: none;
  }
  .daily .plot {
    flex: 1;
    min-height: 0;
    margin-top: 8px;
    display: flex;
    flex-direction: column;
  }

  /* ── mid splits ── */
  .mid {
    display: grid;
    grid-template-columns: 1.5fr 1fr;
    border-bottom: 2px solid var(--ink);
  }
  .mid .bars {
    padding: clamp(13px, 1.3vh, 22px) clamp(22px, 1.8vw, 40px) clamp(15px, 1.5vh, 24px);
    border-right: 2px solid var(--ink);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .mid h3 {
    font: 600 clamp(11px, 0.85vw, 15px) / 1 var(--font-ui);
    letter-spacing: 1.6px;
    text-transform: uppercase;
    margin: 0 0 4px;
    display: flex;
    justify-content: space-between;
  }
  .mid h3 .cnt {
    font: 500 10px / 1 var(--font-mono);
    letter-spacing: 1px;
    opacity: 0.5;
    text-transform: none;
  }
  .clickable {
    cursor: pointer;
  }
  .clickable:hover .nm {
    text-decoration: underline;
  }

  .rel {
    padding: clamp(13px, 1.3vh, 22px) clamp(18px, 1.4vw, 30px) clamp(15px, 1.5vh, 24px);
    display: flex;
    flex-direction: column;
  }
  .obar {
    display: flex;
    height: 18px;
    border: 2px solid var(--ink);
    margin-top: 4px;
  }
  .obar .seg {
    transform-origin: left;
  }
  .oleg {
    margin-top: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .oleg .row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font: 500 11px / 1 var(--font-mono);
    letter-spacing: 0.6px;
  }
  .oleg .row i {
    width: 10px;
    height: 10px;
    align-self: center;
    flex: none;
  }
  .oleg .onm {
    text-transform: uppercase;
    opacity: 0.7;
    flex: 1;
  }
  .oleg .ov {
    font-family: var(--font-disp);
    font-size: 18px;
    letter-spacing: 0.5px;
    font-variant-numeric: tabular-nums;
  }
  .oleg .op {
    opacity: 0.5;
    width: 4ch;
    text-align: right;
  }
  .lat {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px;
    margin-top: auto;
    padding-top: 16px;
  }
  .lat .cell {
    border: 2px solid var(--ink);
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .lat .k {
    font: 600 9px / 1 var(--font-ui);
    letter-spacing: 1.4px;
    text-transform: uppercase;
    opacity: 0.6;
  }
  .lat .v {
    font: 400 clamp(20px, 1.7vw, 34px) / 1 var(--font-disp);
    letter-spacing: -0.5px;
    font-variant-numeric: tabular-nums;
  }

  .side {
    display: flex;
    flex-direction: column;
  }
  .mini {
    padding: clamp(13px, 1.3vh, 22px) clamp(18px, 1.4vw, 30px);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .mini + .mini {
    border-top: 2px solid var(--ink);
  }
  .mrow {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font: 500 11px / 1.2 var(--font-mono);
    letter-spacing: 0.4px;
  }
  .mrow i {
    width: 10px;
    height: 10px;
    align-self: center;
    flex: none;
  }
  .mrow .nm {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mrow b {
    font-family: var(--font-disp);
    font-weight: 400;
    font-size: 17px;
    letter-spacing: 0.5px;
    font-variant-numeric: tabular-nums;
  }
  .mrow .pct {
    opacity: 0.5;
    width: 4ch;
    text-align: right;
  }
  .mrow:hover .nm {
    text-decoration: underline;
  }

  /* ── chats table ── */
  .chats {
    padding: clamp(13px, 1.3vh, 22px) clamp(22px, 1.8vw, 40px) clamp(15px, 1.5vh, 24px);
    border-bottom: 2px solid var(--ink);
  }
  .sec-hd {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    margin-bottom: 12px;
  }
  .sec-hd h2 {
    margin: 0;
    font: 600 clamp(13px, 1vw, 17px) / 1 var(--font-ui);
    letter-spacing: 1.6px;
    text-transform: uppercase;
  }
  .sec-note {
    font: 500 10px / 1 var(--font-mono);
    letter-spacing: 0.8px;
    text-transform: uppercase;
    opacity: 0.5;
  }
  .tw {
    overflow-x: auto;
  }
  .srow {
    cursor: pointer;
  }
  .srow:hover {
    background: var(--hair);
  }
  .srow.open {
    background: rgba(13, 13, 11, 0.05);
  }
  .sid-wrap {
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .tri {
    display: inline-block;
    width: 12px;
    font-size: 10px;
    transition: transform 0.25s cubic-bezier(0.2, 0.7, 0.3, 1);
    opacity: 0.6;
  }
  .tri.open {
    transform: rotate(90deg);
  }
  .sid {
    font-family: var(--font-mono);
    font-weight: 500;
    letter-spacing: 0.5px;
  }
  .copy-btn {
    border: none;
    background: none;
    padding: 0 3px;
    font-size: 11px;
    cursor: pointer;
    opacity: 0.45;
    color: var(--ink);
  }
  .copy-btn:hover {
    opacity: 1;
  }
  .mchips {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }
  .modtag {
    font: 500 9px / 1 var(--font-mono);
    letter-spacing: 0.4px;
    border: 1px solid var(--hair);
    padding: 3px 5px;
    opacity: 0.75;
    white-space: nowrap;
  }
  .modtag.more {
    opacity: 0.45;
  }
  .warn {
    color: var(--mag);
    font-weight: 600;
  }
  .muted {
    opacity: 0.5;
  }

  /* ── expansion timeline ── */
  .xrow td {
    padding: 0 !important;
    border-bottom: none !important;
    background: rgba(13, 13, 11, 0.04);
  }
  .tlwrap {
    overflow: hidden;
    animation: wc-open 0.4s cubic-bezier(0.2, 0.7, 0.3, 1) both;
  }
  @keyframes wc-open {
    from {
      opacity: 0;
      max-height: 0;
    }
    30% {
      opacity: 1;
    }
    to {
      opacity: 1;
      max-height: 420px;
    }
  }
  .tl {
    max-height: 420px;
    overflow-y: auto;
    padding: 6px 18px 10px;
  }
  .call {
    display: grid;
    grid-template-columns: 64px 74px 110px 1fr 70px 64px 70px;
    align-items: baseline;
    gap: 10px;
    padding: 4px 6px;
    font: 500 11px / 1.3 var(--font-mono);
    letter-spacing: 0.3px;
  }
  .call:hover {
    background: var(--hair);
  }
  .call.bad .t,
  .call.bad .m {
    opacity: 0.85;
  }
  .call .t {
    opacity: 0.55;
    font-variant-numeric: tabular-nums;
  }
  .call .ptag {
    color: #fff;
    background: var(--ink);
    font-size: 8.5px;
    letter-spacing: 1px;
    padding: 2px 5px;
    text-align: center;
    align-self: center;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .call .who {
    opacity: 0.6;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .call .who.sub {
    color: var(--blu);
    opacity: 0.85;
  }
  .call .m {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .call .tok,
  .call .dur {
    text-align: right;
    font-variant-numeric: tabular-nums;
    opacity: 0.8;
  }
  .call .out {
    text-align: right;
    text-transform: uppercase;
    color: var(--mag);
    font-size: 9px;
    letter-spacing: 1px;
  }
  .expand-bar {
    display: flex;
    justify-content: center;
    margin-top: 12px;
  }

  /* ── note band ── */
  .noteband {
    border-left: none;
    border-right: none;
    border-bottom: none;
    border-top: 2px solid var(--ink);
    margin-top: auto;
  }

  /* ── empty state ── */
  .empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    justify-content: center;
    gap: 14px;
    padding: 40px clamp(22px, 1.8vw, 40px);
  }
  .empty .e-kick {
    font: 600 11px / 1 var(--font-ui);
    letter-spacing: 2px;
    color: var(--cyn);
  }
  .empty .e-hero {
    font: 400 clamp(36px, 3.4vw, 64px) / 0.95 var(--font-disp);
    letter-spacing: -1px;
    text-transform: uppercase;
  }
  .empty p {
    max-width: 60ch;
    margin: 0;
    font: 400 13px / 1.6 var(--font-mono);
    opacity: 0.65;
  }
  .empty code {
    font-weight: 600;
    opacity: 0.9;
  }

  @media (prefers-reduced-motion: reduce) {
    .tlwrap {
      animation: none;
    }
    .tri {
      transition: none;
    }
  }

  @media (max-width: 900px) {
    .band {
      grid-template-columns: 1fr;
    }
    .hero {
      border-right: none;
      border-bottom: 2px solid var(--ink);
    }
    .hero .n {
      font-size: 72px;
    }
    .mid {
      grid-template-columns: 1fr;
    }
    .mid .bars {
      border-right: none;
      border-bottom: 2px solid var(--ink);
    }
    .call {
      grid-template-columns: 56px 64px 90px 1fr 60px 56px;
    }
    .call .dur {
      display: none;
    }
  }
</style>
