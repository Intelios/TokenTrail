<script lang="ts">
  import AnimatedNumber from "$lib/AnimatedNumber.svelte";
  import Chart from "$lib/Chart.svelte";
  import Section from "./Section.svelte";
  import type { Breakdown, Call, Stats, Usage } from "./types";
  import type { Auras } from "./auras.svelte";
  import { fmtCost, fmtTokens, MODEL_PALETTE } from "$lib/format";
  import { callSwarm, flowSankey, generations, rankBars, rankBarsHeight } from "./charts";
  import { count, duration, ESTIMATED, featureColor, featureLabel } from "./labels";

  let {
    usage,
    stats,
    library,
    auras,
    allUsage = $bindable(),
    days = $bindable(),
    expanded,
    calls,
    callsLoading,
    expand,
    conversationName,
  }: {
    usage: Usage;
    stats: Stats | null;
    library: string;
    auras: Auras;
    allUsage: boolean;
    days: number;
    expanded: string;
    calls: Call[];
    callsLoading: boolean;
    expand: (id: string) => void;
    conversationName: (id: string) => string;
  } = $props();

  const t = $derived(usage.totals);
  const measured = $derived(t.input_tokens + t.output_tokens + t.cache_read_tokens + t.cache_write_tokens);
  const parts = $derived(
    [
      { name: "Input", value: t.input_tokens, color: "#00c2c2" },
      { name: "Output", value: t.output_tokens, color: "#0d0d0b" },
      { name: "Cache read", value: t.cache_read_tokens, color: "#c8e600" },
      { name: "Cache write", value: t.cache_write_tokens, color: "#ff4d00" },
    ].filter((p) => p.value > 0),
  );
  let weight = $state<"events" | "measured_tokens">("events");
  let showCallTable = $state(false);

  const modelColor = (m: string) => {
    const i = usage.models.findIndex((x) => x.name === m);
    return MODEL_PALETTE[(i < 0 ? usage.models.length : i) % MODEL_PALETTE.length]!;
  };
  const characterRows = $derived(
    usage.characters.slice(0, 12).map((r) => {
      // all-libraries view qualifies ids as "library · character"
      const id = r.name.includes(" · ") ? r.name.split(" · ").pop()! : r.name;
      const name = stats?.cast.find((c) => c.character_id === id)?.name ?? id.replace(/\.png$/i, "");
      return {
        name,
        value: r.events,
        color: auras.of(library, id, stats?.colors.characters).fill,
        note: `${fmtTokens(r.measured_tokens)} measured · ${fmtTokens(r.estimated_tokens)} estimated tokens`,
      };
    }),
  );
  const convMax = $derived(Math.max(1, ...usage.conversations.map((c) => c.events)));
  const ownerColor = (id: string) => {
    const chat = stats?.chats.find((c) => c.id === id) ?? stats?.groups.find((c) => c.id === id);
    return chat?.character_id ? auras.of(library, chat.character_id, stats?.colors.characters).fill : "#8a8578";
  };
</script>

{#snippet breakdownTable(title: string, data: Breakdown[], label: (n: string) => string, column = "Name")}
  <div>
    <h3 class="sub">{title}</h3>
    <div class="wc-scroll">
      <table class="wc-table">
        <thead><tr><th>{column}</th><th class="num">Calls</th><th class="num">Measured</th><th class="num">Estimated</th><th class="num">API-equiv.</th></tr></thead>
        <tbody>
          {#each data as r (r.name)}<tr>
              <td>{label(r.name)}</td><td class="num">{count(r.events)}</td>
              <td class="num">{fmtTokens(r.measured_tokens)}</td><td class="num">{fmtTokens(r.estimated_tokens)}</td>
              <td class="num">{r.cost_usd === null ? "—" : fmtCost(r.cost_usd)}</td>
            </tr>{/each}
        </tbody>
      </table>
    </div>
  </div>
{/snippet}

<div class="controls">
  <p>Generation history survives changes to your library.</p>
  <label><input type="checkbox" bind:checked={allUsage} /> All libraries, including unattributed history</label>
  <div class="range">
    {#each [[7, "7D"], [30, "30D"], [90, "90D"], [0, "ALL"]] as [n, label]}<button class:active={days === n} onclick={() => (days = Number(n))}>{label}</button>{/each}
  </div>
</div>

<div class="strip up">
  <div class="cell"><span>Generations</span><strong><AnimatedNumber value={t.events} format={count} /></strong></div>
  <div class="cell lit"><span>Measured tokens</span><strong>{fmtTokens(measured)}</strong></div>
  <div class="cell est"><span>Estimated completion tokens</span><strong>{fmtTokens(t.estimated_tokens)}</strong><small class="tag">Never priced</small></div>
  <div class="cell"><span>API-equivalent cost estimate</span><strong>{t.cost_usd === null ? "—" : fmtCost(t.cost_usd)}</strong></div>
  <div class="cell"><span>Average duration</span><strong>{duration(t.avg_duration_ms)}</strong></div>
  <div class="cell"><span>First token</span><strong>{duration(t.avg_ttft_ms)}</strong></div>
</div>
{#if measured > 0}
  <div class="composition" aria-label="Measured token composition">
    {#each parts as p, i (p.name)}<i class="gw" style="flex:{p.value};background:{p.color};animation-delay:{i * 80}ms" title="{p.name} · {fmtTokens(p.value)}"></i>{/each}
  </div>
  <div class="wc-legend">
    {#each parts as p (p.name)}<span><i style="background:{p.color}"></i>{p.name} · {fmtTokens(p.value)} · {((p.value / measured) * 100).toFixed(0)}%</span>{/each}
  </div>
{/if}
<p class="wc-note">
  List-price estimate, not actual subscription spend. {count(t.priced_events)} of {count(t.measured_events)} measured generations priced. Estimated tokens are never priced.
</p>

{#if t.events === 0}
  <div class="empty">
    <h2>No recorded generations in this range</h2>
    <p>Enable “Record usage for TokenTrail” in WackChatter, or choose a wider range. Existing transcript usage is imported automatically.</p>
  </div>
{:else}
  <Section index="01" kicker="Generations" title="The work behind your stories" note="Daily measured tokens by feature · UTC days · Dashed violet bars are estimates, kept apart · Line is the running API-equivalent cost estimate">
    <Chart option={generations(usage.daily_features)} height={330} />
    {#snippet table()}
      {@render breakdownTable("Daily", [...usage.daily].sort((a, b) => b.name.localeCompare(a.name)), (n) => n, "Day")}
    {/snippet}
  </Section>

  <Section index="02" kicker="Flow" title="Feature → model → outcome" note={weight === "events" ? "Weighted by calls" : "Weighted by measured tokens · Estimated calls carry no weight here"}>
    {#snippet aside()}
      <div class="range">
        <button class:active={weight === "events"} onclick={() => (weight = "events")}>Calls</button>
        <button class:active={weight === "measured_tokens"} onclick={() => (weight = "measured_tokens")}>Tokens</button>
      </div>
    {/snippet}
    <Chart option={flowSankey(usage.flows, weight, modelColor)} height={480} />
    {#snippet table()}
      <div class="tables">
        {@render breakdownTable("Features", usage.features, featureLabel)}
        {@render breakdownTable("Models", usage.models, (n) => n)}
        {@render breakdownTable("Providers", usage.providers, (n) => n)}
        {@render breakdownTable("Outcomes", usage.outcomes, (n) => n)}
      </div>
    {/snippet}
  </Section>

  {#if characterRows.length}
    <Section index="03" kicker="Characters" title="Who the calls were for" note="Calls by the character a generation was attributed to · Top 12">
      <Chart option={rankBars(characterRows, count)} height={rankBarsHeight(characterRows.length)} />
      {#snippet table()}
        {@render breakdownTable("Characters", usage.characters, (n) => stats?.cast.find((c) => c.character_id === n)?.name ?? n)}
      {/snippet}
    </Section>
  {/if}

  <Section index="04" kicker="Conversations" title="Follow the generations" note="Expand a conversation to see every call it made, in time">
    <div class="conv-list">
      {#each usage.conversations as c (c.name)}
        <div class="conv" class:open={expanded === c.name}>
          <button class="conv-head" onclick={() => expand(c.name)} aria-expanded={expanded === c.name}>
            <span class="sign">{expanded === c.name ? "−" : "+"}</span>
            <span class="conv-name">{conversationName(c.name)}</span>
            <span class="conv-bar"><i class="gw" style="width:{(c.events / convMax) * 100}%;background:{ownerColor(c.name)}"></i></span>
            <span class="conv-meta"><b>{count(c.events)}</b> calls · {fmtTokens(c.measured_tokens)} measured{#if c.estimated_tokens} · <em>{fmtTokens(c.estimated_tokens)} estimated</em>{/if}</span>
          </button>
          {#if expanded === c.name}
            <div class="conv-body up">
              {#if callsLoading}<p class="wc-note">Loading generations…</p>
              {:else if calls.length}
                <div class="conv-tools">
                  <span class="wc-note">● completed · ◆ cancelled · ✕ failed · hollow violet = estimated tokens · size = tokens</span>
                  <button class="toggle" class:on={showCallTable} aria-pressed={showCallTable} onclick={() => (showCallTable = !showCallTable)}>{showCallTable ? "Hide table" : "Show table"}</button>
                </div>
                <Chart option={callSwarm(calls)} height={220} />
                {#if showCallTable}
                  <div class="wc-scroll">
                    <table class="wc-table">
                      <thead><tr><th>Time</th><th>Feature</th><th>Model</th><th>Outcome</th><th class="num">Measured</th><th class="num">Estimated</th><th class="num">Duration / first token</th></tr></thead>
                      <tbody>
                        {#each calls as call (call.id)}<tr>
                            <td>{new Date(call.ts).toLocaleString()}</td>
                            <td><i class="wc-swatch" style="background:{featureColor(call.purpose)}"></i>{featureLabel(call.purpose)}</td>
                            <td>{call.model}</td><td>{call.outcome}</td>
                            <td class="num">{fmtTokens(call.measured_tokens)}</td>
                            <td class="num" style={call.estimated_tokens ? `color:${ESTIMATED}` : ""}>{fmtTokens(call.estimated_tokens)}</td>
                            <td class="num">{duration(call.duration_ms)} / {duration(call.ttft_ms)}</td>
                          </tr>{/each}
                      </tbody>
                    </table>
                  </div>
                {/if}
              {:else}<p class="wc-note">No generations in this range.</p>{/if}
            </div>
          {/if}
        </div>
      {/each}
    </div>
  </Section>
{/if}

<style>
  .controls {
    display: flex;
    align-items: center;
    gap: 18px;
    flex-wrap: wrap;
    margin-bottom: 20px;
  }
  .controls p {
    margin: 0 auto 0 0;
    font-size: 13px;
  }
  .controls label {
    font-size: 12px;
  }
  .range {
    display: flex;
  }
  .range button {
    background: transparent;
    border: 2px solid var(--ink);
    margin-left: -2px;
    padding: 7px 12px;
    font: 600 11px var(--font-mono);
    color: var(--ink);
    cursor: pointer;
    border-radius: 0;
  }
  .range button.active,
  .range button:hover {
    background: var(--ink);
    color: var(--bone);
  }
  .strip {
    display: grid;
    grid-template-columns: repeat(6, minmax(0, 1fr));
    border: 2px solid var(--ink);
    gap: 2px;
    background: var(--ink);
  }
  .cell {
    background: var(--bone);
    padding: 16px 14px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-width: 0;
  }
  .cell > span {
    font: 9.5px/1.4 var(--font-mono);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    min-height: 26px;
  }
  .cell strong {
    font: 36px/1 var(--font-disp);
  }
  .cell.lit {
    background: var(--acd);
  }
  .cell.est strong {
    color: var(--vio);
  }
  .tag {
    font: 9px var(--font-mono);
    text-transform: uppercase;
    letter-spacing: 1px;
    border: 1px solid currentColor;
    padding: 2px 5px;
    align-self: flex-start;
    color: var(--vio);
  }
  .composition {
    display: flex;
    height: 22px;
    border: 2px solid var(--ink);
    border-top: 0;
    gap: 2px;
    background: var(--ink);
  }
  .composition i {
    display: block;
    min-width: 3px;
  }
  .wc-note {
    margin: 10px 0 0;
  }
  .empty {
    padding: 60px 30px;
    text-align: center;
    border: 2px solid var(--ink);
    margin-top: 24px;
  }
  .empty h2 {
    font: 30px var(--font-disp);
    margin: 0 0 10px;
  }
  .sub {
    font: 600 13px var(--font-ui);
    text-transform: uppercase;
    letter-spacing: 1px;
    margin: 0 0 10px;
  }
  .tables {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 26px 30px;
  }
  .conv-list {
    border-top: 2px solid var(--ink);
    max-height: 780px;
    overflow: auto;
  }
  .conv {
    border-bottom: 1px solid var(--hair);
  }
  .conv.open {
    border-bottom: 2px solid var(--ink);
  }
  .conv-head {
    width: 100%;
    display: grid;
    grid-template-columns: 18px minmax(180px, 1.2fr) minmax(120px, 1fr) auto;
    align-items: center;
    gap: 16px;
    padding: 14px 4px;
    border: 0;
    background: transparent;
    text-align: left;
    cursor: pointer;
    font: inherit;
    color: var(--ink);
  }
  .conv-head:hover {
    background: rgba(13, 13, 11, 0.05);
  }
  .sign {
    font: 18px var(--font-mono);
  }
  .conv-name {
    font: 600 14px var(--font-ui);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .conv-bar {
    height: 10px;
    background: var(--hair);
  }
  .conv-bar i {
    display: block;
    height: 100%;
  }
  .conv-meta {
    font: 11px var(--font-mono);
    white-space: nowrap;
  }
  .conv-meta b {
    font: 16px var(--font-disp);
    font-weight: 400;
  }
  .conv-meta em {
    color: var(--vio);
    font-style: normal;
  }
  .conv-body {
    padding: 0 4px 18px;
  }
  .conv-body :global(.wc-scroll) {
    margin-top: 14px;
  }
  .conv-tools {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
  }
  .conv-tools .wc-note {
    margin: 0;
  }
  .toggle {
    background: transparent;
    border: 2px solid var(--ink);
    padding: 6px 10px;
    font: 600 10px var(--font-mono);
    letter-spacing: 1px;
    text-transform: uppercase;
    color: var(--ink);
    cursor: pointer;
    border-radius: 0;
  }
  .toggle:hover,
  .toggle.on {
    background: var(--ink);
    color: var(--bone);
  }
  @media (max-width: 1100px) {
    .strip {
      grid-template-columns: repeat(3, minmax(0, 1fr));
    }
    .tables {
      grid-template-columns: 1fr;
    }
    .conv-head {
      grid-template-columns: 18px minmax(0, 1fr) auto;
    }
    .conv-bar {
      display: none;
    }
  }
</style>
