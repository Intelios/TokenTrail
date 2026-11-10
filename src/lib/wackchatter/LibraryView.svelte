<script lang="ts">
  import Chart from "$lib/Chart.svelte";
  import Spark from "$lib/Spark.svelte";
  import Portrait from "./Portrait.svelte";
  import Section from "./Section.svelte";
  import ProfileView from "./ProfileView.svelte";
  import TotalsStrip from "./TotalsStrip.svelte";
  import type { Stats } from "./types";
  import type { Auras } from "./auras.svelte";
  import { fmtTokens, MODEL_PALETTE } from "$lib/format";
  import {
    busiestDay,
    foldActivity,
    foldDays,
    foldWeekHour,
    streaks,
  } from "./buckets";
  import {
    calendarHeat,
    calendarHeight,
    castEras,
    meter,
    modelVoices,
    personaMatrix,
    rankBars,
    rankBarsHeight,
    rhythmClock,
    type Lane,
  } from "./charts";
  import { count, date, duration, percent } from "./labels";

  let {
    library,
    stats,
    character,
    group,
    auras,
    accent,
    navigate,
    open,
  }: {
    library: string;
    stats: Stats;
    character: string | null;
    group: string | null;
    auras: Auras;
    /** The page accent: the character's aura on a profile, lime otherwise. */
    accent: { fill: string; ink: string };
    navigate: (patch: Record<string, string | null>) => void;
    open: (chat?: string | null, char?: string | null) => void;
  } = $props();

  const scoped = $derived(Boolean(character || group));
  let allCast = $state(false);
  const CAST_PREVIEW = 12;
  const aura = (id: string) => auras.of(library, id, stats.colors.characters);
  const nameOf = (id: string) =>
    stats.cast.find((c) => c.character_id === id)?.name ?? id.replace(/\.png$/i, "");

  const calendar = $derived(foldDays(stats.hours));
  const streak = $derived(streaks(calendar));
  const busiest = $derived(busiestDay(calendar));
  const top = $derived(stats.cast[0]);

  const monthly = $derived(foldActivity(stats.cast_activity, "month"));
  const castSparks = $derived(
    new Map(
      [...monthly.series].map(([id, v]) => [id, v.slice(-12)] as const),
    ),
  );

  const eraLanes = $derived.by<Lane[]>(() => {
    const ranked = stats.cast.filter((c) => monthly.series.has(c.character_id));
    const head = ranked.slice(0, 8).map((c) => ({
      id: c.character_id,
      name: c.name,
      color: aura(c.character_id).fill,
      values: monthly.series.get(c.character_id)!,
    }));
    const rest = ranked.slice(8);
    if (rest.length) {
      const values = new Array<number>(monthly.periods.length).fill(0);
      for (const c of rest)
        monthly.series.get(c.character_id)!.forEach((v, i) => (values[i]! += v));
      head.push({
        id: "others",
        name: `+${rest.length} others`,
        color: "#b8b2a4",
        values,
      });
    }
    return head;
  });

  const weekly = $derived(foldActivity(stats.model_activity, "week"));
  const modelColor = (name: string) => {
    const i = stats.models.findIndex((m) => m.model === name);
    return MODEL_PALETTE[(i < 0 ? stats.models.length : i) % MODEL_PALETTE.length]!;
  };
  const voiceLanes = $derived.by<Lane[]>(() =>
    [...weekly.series]
      .map(([id, values]) => ({ id, name: id, color: modelColor(id), values }))
      .sort(
        (a, b) =>
          stats.models.findIndex((m) => m.model === a.id) -
          stats.models.findIndex((m) => m.model === b.id),
      ),
  );
  // The same model can arrive from more than one provider; the kept rate is a
  // property of the model, so those rows add up before the share is taken.
  const keptRows = $derived.by(() => {
    const byModel = new Map<string, { kept: number; swipes: number }>();
    for (const m of stats.models) {
      const row = byModel.get(m.model) ?? { kept: 0, swipes: 0 };
      row.kept += m.kept_swipes;
      row.swipes += m.swipes;
      byModel.set(m.model, row);
    }
    return [...byModel]
      .filter(([, row]) => row.swipes > 0)
      .sort((a, b) => b[1].swipes - a[1].swipes)
      .map(([name, row]) => ({
        name,
        value: (row.kept / row.swipes) * 100,
        color: modelColor(name),
        note: `${count(row.kept)} of ${count(row.swipes)} swipes kept`,
      }));
  });

  const personaName = (id: string | null) =>
    stats.personas.find((p) => p.persona_id === id)?.name ??
    (id === null ? "Unknown persona (legacy)" : id === "" ? "No persona" : "Deleted persona");
  const matrix = $derived.by(() => {
    const personaIds = [...new Set(stats.persona_cast.map((p) => p.persona_id))].sort(
      (a, b) =>
        stats.persona_cast.filter((p) => p.persona_id === b).reduce((t, p) => t + p.messages, 0) -
        stats.persona_cast.filter((p) => p.persona_id === a).reduce((t, p) => t + p.messages, 0),
    );
    const present = new Set(stats.persona_cast.map((p) => p.character_id));
    const columns = stats.cast
      .filter((c) => present.has(c.character_id))
      .slice(0, 10)
      .map((c) => ({ id: c.character_id, name: c.name, color: aura(c.character_id).fill }));
    if (present.has("group")) columns.push({ id: "group", name: "Groups", color: "#8a8578" });
    const cells: [number, number, number][] = [];
    personaIds.forEach((pid, y) =>
      columns.forEach((col, x) => {
        const hit = stats.persona_cast.find(
          (p) => p.persona_id === pid && p.character_id === col.id,
        );
        cells.push([x, y, hit?.messages ?? 0]);
      }),
    );
    return { personas: personaIds.map(personaName), columns, cells };
  });
  const personaRows = $derived(
    stats.personas
      .filter((p) => p.messages > 0)
      .map((p) => ({ name: p.name, value: p.messages, color: accent.fill })),
  );

  const groupRows = $derived(
    stats.groups.map((g) => ({ name: g.title, value: g.replies, color: accent.fill })),
  );

  const h = $derived(stats.habits);
  const longest = $derived(stats.chats[0]);
  const lengthMax = $derived(Math.max(1, h.avg_user_chars, h.avg_reply_chars));
  const chatMax = $derived(Math.max(1, ...stats.chats.map((c) => c.messages)));
</script>

{#if scoped}
  <ProfileView {library} {stats} {character} {group} {auras} {accent} {navigate} {open} />
{:else}
  <TotalsStrip {stats} />
{/if}

{#if !scoped && top}
  {@const a = aura(top.character_id)}
  <button
    class="hero up"
    style="--hero:{a.fill};--hero-ink:{a.ink}"
    onclick={() => navigate({ character: top.character_id, group: null })}
  >
    <div class="hero-portrait">
      <Portrait {library} id={top.character_id} name={top.name} tint={a.fill} letter={110} />
    </div>
    <div class="hero-body">
      <p class="eyebrow">Most played · By recorded replies</p>
      <h2>{top.name}</h2>
      <div class="hero-facts">
        <div><b>{count(top.replies)}</b><span>Replies</span></div>
        <div><b>{count(top.chats)}</b><span>Conversations</span></div>
        <div><b>{date(top.first_chat)}</b><span>Together since</span></div>
      </div>
      <div class="hero-foot">
        <span class="cta">Explore your history together →</span>
        <div class="hero-spark">
          <span>Replies · last 12 months</span>
          <Spark values={castSparks.get(top.character_id) ?? []} width={400} height={64} color="var(--ink)" stroke={2} />
        </div>
      </div>
    </div>
    <b class="hero-number">01</b>
  </button>
{/if}

{#if !scoped && stats.cast.length}
  <Section index="01" kicker="The cast" title="Who you spend it with" note="Each character keeps the colour WackChatter gives them: a pinned colour first, else their portrait.">
    <div class="cast-grid">
      {#each allCast ? stats.cast : stats.cast.slice(0, CAST_PREVIEW) as c, i (c.character_id)}
        {@const a = aura(c.character_id)}
        <button
          class="cast-card up"
          style="--card:{a.fill};--card-ink:{a.ink};animation-delay:{Math.min(i, 12) * 40}ms"
          onclick={() => navigate({ character: c.character_id, group: null })}
        >
          <div class="cast-image">
            <Portrait {library} id={c.character_id} name={c.name} tint={a.fill} />
            <span>{String(i + 1).padStart(2, "0")}</span>
          </div>
          <div class="cast-body">
            <h3>{c.name}</h3>
            <p>{count(c.replies)} replies · {count(c.chats)} chats</p>
            <Spark values={castSparks.get(c.character_id) ?? []} width={150} height={22} color={a.ink} />
          </div>
        </button>
      {/each}
    </div>
    {#if stats.cast.length > CAST_PREVIEW}
      <button class="wc-btn cast-more" onclick={() => (allCast = !allCast)}>
        {allCast ? `Show the top ${CAST_PREVIEW}` : `Show all ${count(stats.cast.length)} characters`}
      </button>
    {/if}
    {#if eraLanes.length && monthly.periods.length > 1}
      <h3 class="sub">Cast eras <small>Replies per month · each lane scaled to its own peak</small></h3>
      <Chart
        option={castEras(monthly.periods, eraLanes)}
        height={eraLanes.length * 46 + 44}
        onclick={(p) => p.data?.lane && p.data.lane !== "others" && navigate({ character: p.data.lane, group: null })}
      />
    {/if}
    {#snippet table()}
      <div class="wc-scroll">
        <table class="wc-table">
          <thead><tr><th>Character</th><th class="num">Replies</th><th class="num">Chats</th><th class="num">Measured tokens</th><th class="num">Estimated tokens</th><th>First</th><th>Last</th></tr></thead>
          <tbody>
            {#each stats.cast as c (c.character_id)}<tr>
                <td><i class="wc-swatch" style="background:{aura(c.character_id).fill}"></i>{c.name}</td>
                <td class="num">{count(c.replies)}</td><td class="num">{count(c.chats)}</td>
                <td class="num">{fmtTokens(c.measured_tokens)}</td><td class="num">{fmtTokens(c.estimated_tokens)}</td>
                <td>{date(c.first_chat)}</td><td>{date(c.last_chat)}</td>
              </tr>{/each}
          </tbody>
        </table>
      </div>
    {/snippet}
  </Section>
{/if}

<Section index="02" kicker="Your rhythm" title="When you play" note="Your local calendar · Every message, all time">
  <div class="rhythm">
    <div class="calendar">
      {#if calendar.length}<Chart option={calendarHeat(calendar)} height={calendarHeight(calendar)} />{:else}<p class="wc-note">No dated messages yet.</p>{/if}
    </div>
    <div class="clock">
      <Chart option={rhythmClock(foldWeekHour(stats.hours), accent.fill)} height={340} />
      <p class="wc-note">Hour around the dial · Monday innermost, Sunday outermost · Dot size = messages</p>
    </div>
  </div>
  <div class="wc-badges">
    <div class="wc-badge lit"><b>{streak.current}d</b><span>Current streak</span></div>
    <div class="wc-badge"><b>{streak.longest}d</b><span>Longest streak</span></div>
    <div class="wc-badge"><b>{count(calendar.length)}</b><span>Active days</span></div>
    <div class="wc-badge">
      <b>{busiest ? date(busiest.day) : "—"}</b><span>Busiest day{busiest ? ` · ${count(busiest.count)} messages` : ""}</span>
    </div>
  </div>
</Section>

<Section index="03" kicker="Model voices" title="Who is doing the writing" note="Swipes per model, by local week · Kept rate is the share of a model's swipes still showing in your chats">
  {#if voiceLanes.length && weekly.periods.length > 1}
    <div class="voices">
      <div>
        <Chart option={modelVoices(weekly.periods, voiceLanes)} height={300} />
        <div class="wc-legend">
          {#each voiceLanes as l (l.id)}<span><i style="background:{l.color}"></i>{l.name}</span>{/each}
        </div>
      </div>
      <div>
        <h3 class="sub">Kept rate <small>Share of swipes still showing</small></h3>
        <div class="kept">
          {#each keptRows as r (r.name)}
            <div class="kept-row">
              <div class="kept-head"><span>{r.name}</span><b>{r.value.toFixed(0)}%</b></div>
              <div class="kept-bar"><i class="gw" style="width:{r.value}%;background:{r.color}"></i></div>
            </div>
          {/each}
        </div>
      </div>
    </div>
  {:else if stats.models.length}
    <div class="kept">
      {#each keptRows as r (r.name)}
        <div class="kept-row">
          <div class="kept-head"><span>{r.name}</span><b>{r.value.toFixed(0)}%</b></div>
          <div class="kept-bar"><i class="gw" style="width:{r.value}%;background:{r.color}"></i></div>
        </div>
      {/each}
    </div>
  {:else}<p class="wc-note">No generated replies yet.</p>{/if}
  {#snippet table()}
    <div class="wc-scroll">
      <table class="wc-table">
        <thead><tr><th>Model / provider</th><th class="num">Swipes</th><th class="num">Kept</th><th class="num">Measured tokens</th><th class="num">Estimated tokens</th><th class="num">Latency</th><th class="num">Measured tok/s</th><th class="num">Est. tok/s</th></tr></thead>
        <tbody>
          {#each stats.models as m (m.model + m.provider)}<tr>
              <td><i class="wc-swatch" style="background:{modelColor(m.model)}"></i>{m.model}<small>{m.provider}</small></td>
              <td class="num">{count(m.swipes)}</td>
              <td class="num">{percent(m.kept_swipes, m.swipes)}</td>
              <td class="num">{fmtTokens(m.measured_tokens)}</td>
              <td class="num">{fmtTokens(m.estimated_tokens)}</td>
              <td class="num">{duration(m.timed_swipes ? m.total_ms / m.timed_swipes : null)}</td>
              <td class="num">{m.measured_ms > 0 ? ((m.measured_timed_tokens * 1000) / m.measured_ms).toFixed(1) : "—"}</td>
              <td class="num">{m.estimated_ms > 0 ? ((m.estimated_timed_tokens * 1000) / m.estimated_ms).toFixed(1) : "—"}</td>
            </tr>{/each}
        </tbody>
      </table>
    </div>
  {/snippet}
</Section>

<Section index="04" kicker="Personas" title={scoped ? "Who you show up as" : "Who you are, with whom"} note={scoped ? "Your messages by persona" : "Your messages by persona and by the character whose chat it was · Group chats share one column"}>
  {#if !scoped && matrix.cells.length}
    <Chart
      option={personaMatrix(matrix.personas, matrix.columns, matrix.cells)}
      height={matrix.personas.length * 46 + 70}
      onclick={(p) => {
        const col = matrix.columns[p.value?.[0]];
        if (col && col.id !== "group") navigate({ character: col.id, group: null });
      }}
    />
  {:else if personaRows.length}
    <Chart option={rankBars(personaRows, count)} height={rankBarsHeight(personaRows.length)} />
  {:else}<p class="wc-note">No persona activity yet.</p>{/if}
  {#snippet table()}
    <div class="wc-scroll">
      <table class="wc-table">
        <thead><tr><th>Persona</th><th class="num">Messages</th><th class="num">Chats</th></tr></thead>
        <tbody>
          {#each stats.personas as p (p.persona_id ?? p.name)}<tr><td>{p.name}</td><td class="num">{count(p.messages)}</td><td class="num">{count(p.chats)}</td></tr>{/each}
        </tbody>
      </table>
    </div>
  {/snippet}
</Section>

<Section index="05" kicker="Player card" title="How you play">
  <div class="player">
    <div class="gauge">
      <Chart option={meter(h.eligible_replies ? (h.rerolled_replies / h.eligible_replies) * 100 : 0, accent.fill, "REPLIES REROLLED")} height={210} />
      <p><b>{count(h.rerolls)}</b> rerolls in all</p>
    </div>
    <div class="lengths">
      <h3 class="sub">You vs. them <small>Average characters per message</small></h3>
      <div class="len-row">
        <span>You</span>
        <div class="len-bar"><i class="gw" style="width:{(h.avg_user_chars / lengthMax) * 100}%;background:var(--ink)"></i></div>
        <b>{count(h.avg_user_chars)}</b>
      </div>
      <div class="len-row">
        <span>Replies</span>
        <div class="len-bar"><i class="gw" style="width:{(h.avg_reply_chars / lengthMax) * 100}%;background:{accent.fill}"></i></div>
        <b>{count(h.avg_reply_chars)}</b>
      </div>
      <p class="ratio">
        {#if h.avg_user_chars > 0}They write <b>{(h.avg_reply_chars / h.avg_user_chars).toFixed(1)}×</b> as much as you{:else}No messages from you yet{/if}
        · Longest reply <b>{count(h.longest_reply_chars)}</b> characters
      </p>
    </div>
    <div class="gauge">
      <Chart option={meter(stats.totals.swipes ? (h.reasoning_swipes / stats.totals.swipes) * 100 : 0, "#7c5cff", "SWIPES WITH REASONING")} height={210} />
      <p><b>{count(h.reasoning_swipes)}</b> reasoned swipes</p>
    </div>
  </div>
  <div class="wc-badges">
    <div class="wc-badge"><b>{count(h.branched_chats)}</b><span>Branched chats</span></div>
    <div class="wc-badge"><b>{count(h.deleted_chats)}</b><span>Chats in trash</span></div>
    {#if !scoped}<div class="wc-badge"><b>{count(stats.co_creator_sessions)}</b><span>Co-Creator sessions</span></div>{/if}
    <div class="wc-badge"><b>{count(h.timed_swipes)}</b><span>Timed swipes</span></div>
    <div class="wc-badge">
      <b>{longest ? count(longest.messages) : "—"}</b><span>Longest conversation{longest ? ` · ${longest.title}` : ""}</span>
    </div>
  </div>
</Section>

<Section index="06" kicker="Conversations" title={group ? "This group" : character ? "The long conversations" : "Your conversations"}>
  {#if stats.chats.length}
    <div class="chats">
      {#each stats.chats as c, i (c.id)}
        {@const owner = c.character_id}
        {@const a = owner ? aura(owner) : null}
        <div class="chat-row">
          <div class="face" style="--chip:{a?.fill ?? '#8a8578'}">
            {#if owner}<Portrait {library} id={owner} name={nameOf(owner)} tint={a?.fill} letter={16} />{:else}<span class="grp">G</span>{/if}
          </div>
          <div class="chat-main">
            <div class="chat-title">
              {#if c.kind === "group" && !group}<button class="link" onclick={() => navigate({ group: c.id, character: null })}>{c.title}</button><small class="tag">Group</small>
              {:else}<span>{c.title}</span>{#if c.kind === "group"}<small class="tag">Group</small>{/if}{/if}
              {#if owner && !character}<small class="who">{nameOf(owner)}</small>{/if}
            </div>
            <div class="chat-bar"><i class="gw" style="width:{(c.messages / chatMax) * 100}%;background:{a?.fill ?? '#8a8578'};animation-delay:{Math.min(i, 20) * 25}ms"></i></div>
          </div>
          <div class="chat-meta"><b>{count(c.messages)}</b><span>messages</span></div>
          <div class="chat-meta"><b>{date(c.modified)}</b><span>last spoke</span></div>
          <button class="wc-btn open" onclick={() => open(c.id)}>Open ↗</button>
        </div>
      {/each}
    </div>
  {:else}<p class="wc-note">No direct conversations yet.</p>{/if}
  {#snippet table()}
    <div class="wc-scroll">
      <table class="wc-table">
        <thead><tr><th>Conversation</th><th class="num">Messages</th><th class="num">Replies</th><th class="num">Measured tokens</th><th class="num">Estimated tokens</th><th>Created</th><th>Last spoke</th></tr></thead>
        <tbody>
          {#each stats.chats as c (c.id)}<tr>
              <td>{c.title}{#if c.kind === "group"}<small>GROUP</small>{/if}</td>
              <td class="num">{count(c.messages)}</td><td class="num">{count(c.replies)}</td>
              <td class="num">{fmtTokens(c.measured_tokens)}</td><td class="num">{fmtTokens(c.estimated_tokens)}</td>
              <td>{date(c.created)}</td><td>{date(c.modified)}</td>
            </tr>{/each}
        </tbody>
      </table>
    </div>
  {/snippet}
</Section>

{#if character && stats.groups.length}
  <Section index="07" kicker="Group participation" title="In the ensemble" note={`Replies and tokens belong to ${stats.name}. Shared user messages and time stay with the group.`}>
    <div class="narrow">
      <Chart
        option={rankBars(groupRows, count)}
        height={rankBarsHeight(groupRows.length)}
        onclick={(p) => {
          const g = stats.groups[stats.groups.length - 1 - p.dataIndex];
          if (g) navigate({ group: g.id, character: null });
        }}
      />
    </div>
    {#snippet table()}
      <div class="wc-scroll">
        <table class="wc-table">
          <thead><tr><th>Group</th><th class="num">Character replies</th><th class="num">Measured tokens</th><th class="num">Estimated tokens</th><th></th></tr></thead>
          <tbody>
            {#each stats.groups as g (g.id)}<tr>
                <td><button class="link" onclick={() => navigate({ group: g.id, character: null })}>{g.title}</button></td>
                <td class="num">{count(g.replies)}</td><td class="num">{fmtTokens(g.measured_tokens)}</td><td class="num">{fmtTokens(g.estimated_tokens)}</td>
                <td><button class="wc-btn" onclick={() => open(g.id)}>Open ↗</button></td>
              </tr>{/each}
          </tbody>
        </table>
      </div>
    {/snippet}
  </Section>
{/if}

<style>
  .hero {
    display: flex;
    align-items: stretch;
    width: 100%;
    text-align: left;
    padding: 0;
    margin: 30px 0 0;
    border: 2px solid var(--ink);
    background: var(--hero);
    color: var(--ink);
    cursor: pointer;
    position: relative;
    overflow: hidden;
    font: inherit;
  }
  .hero:hover .cta {
    background: var(--ink);
    color: var(--bone);
  }
  .hero-portrait {
    width: 230px;
    min-height: 290px;
    flex: none;
    border-right: 2px solid var(--ink);
  }
  .hero-body {
    padding: 26px 30px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    z-index: 1;
    flex: 1;
    min-width: 0;
  }
  .eyebrow {
    font: 11px var(--font-mono);
    letter-spacing: 1.5px;
    text-transform: uppercase;
    margin: 0;
  }
  .hero h2 {
    font: 76px/0.9 var(--font-disp);
    margin: 0;
    text-transform: uppercase;
  }
  .hero-facts {
    display: flex;
    gap: 34px;
  }
  .hero-facts div {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .hero-facts b {
    font: 26px var(--font-disp);
  }
  .hero-facts span,
  .hero-spark span {
    font: 9.5px var(--font-mono);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .hero-foot {
    display: flex;
    align-items: end;
    justify-content: space-between;
    gap: 30px;
    margin-top: auto;
  }
  .hero-spark {
    display: flex;
    flex-direction: column;
    gap: 4px;
    align-items: flex-end;
    flex: none;
  }
  .cta {
    font: 600 11px var(--font-mono);
    text-transform: uppercase;
    letter-spacing: 1px;
    border: 2px solid var(--ink);
    padding: 8px 12px;
    align-self: flex-start;
  }
  .hero-number {
    position: absolute;
    right: 24px;
    top: -26px;
    font: 200px/1 var(--font-disp);
    color: var(--hero-ink);
    opacity: 0.22;
  }

  .cast-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(170px, 1fr));
    gap: 14px;
    margin-bottom: 22px;
  }
  .cast-more {
    margin: 0 0 22px;
  }
  .cast-card {
    text-align: left;
    padding: 0;
    border: 2px solid var(--ink);
    background: var(--bone);
    cursor: pointer;
    font: inherit;
    color: var(--ink);
    transition: transform 0.15s;
  }
  .cast-card:hover {
    transform: translate(-3px, -3px);
    box-shadow: 5px 5px 0 var(--card);
  }
  .cast-image {
    height: 170px;
    position: relative;
    border-bottom: 6px solid var(--card);
  }
  .cast-image > span {
    position: absolute;
    bottom: 0;
    left: 0;
    background: var(--ink);
    color: var(--bone);
    padding: 3px 9px;
    font: 22px var(--font-disp);
  }
  .cast-body {
    padding: 10px 12px 12px;
  }
  .cast-body h3 {
    font: 600 16px/1.2 var(--font-ui);
    margin: 0 0 4px;
    display: -webkit-box;
    line-clamp: 2;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    min-height: 2.4em;
  }
  .cast-body p {
    font: 10.5px var(--font-mono);
    margin: 0 0 8px;
    color: var(--card-ink);
  }
  .narrow {
    max-width: 780px;
  }
  .sub {
    font: 600 13px var(--font-ui);
    text-transform: uppercase;
    letter-spacing: 1px;
    margin: 0 0 10px;
  }
  .sub small {
    font: 10px var(--font-mono);
    text-transform: none;
    letter-spacing: 0;
    color: var(--dim);
    margin-left: 10px;
  }

  .rhythm {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 360px;
    gap: 26px;
    align-items: start;
    margin-bottom: 22px;
  }
  .clock .wc-note {
    text-align: center;
    margin: 0;
  }

  .voices {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 340px;
    gap: 30px;
    align-items: start;
  }

  .kept {
    display: grid;
    gap: 12px;
  }
  .kept-row {
    display: grid;
    gap: 5px;
    min-width: 0;
  }
  .kept-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 12px;
    font: 10.5px var(--font-mono);
  }
  .kept-head span {
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .kept-head b {
    font: 20px/1 var(--font-disp);
    font-weight: 400;
    flex: none;
  }
  .kept-bar {
    height: 12px;
    border: 2px solid var(--ink);
  }
  .kept-bar i {
    display: block;
    height: 100%;
  }
  .player {
    display: grid;
    grid-template-columns: 260px minmax(0, 1fr) 260px;
    gap: 30px;
    align-items: center;
    margin-bottom: 22px;
  }
  .gauge {
    min-width: 0;
  }
  .gauge p {
    text-align: center;
    font: 11px var(--font-mono);
    margin: -20px 0 0;
  }
  .gauge b,
  .ratio b {
    font-family: var(--font-disp);
    font-size: 16px;
    font-weight: 400;
  }
  .len-row {
    display: grid;
    grid-template-columns: 70px minmax(0, 1fr) 70px;
    align-items: center;
    gap: 12px;
    margin: 12px 0;
    font: 11px var(--font-mono);
    text-transform: uppercase;
  }
  .len-row b {
    font: 24px var(--font-disp);
    text-align: right;
  }
  .len-bar {
    height: 30px;
    border: 2px solid var(--ink);
  }
  .len-bar i {
    display: block;
    height: 100%;
  }
  .ratio {
    font: 11px/1.6 var(--font-mono);
    color: var(--dim);
  }

  .chats {
    border-top: 2px solid var(--ink);
  }
  .chat-row {
    display: grid;
    grid-template-columns: 40px minmax(0, 1fr) 90px 120px auto;
    gap: 16px;
    align-items: center;
    padding: 10px 0;
    border-bottom: 1px solid var(--hair);
  }
  .face {
    width: 40px;
    height: 48px;
    border: 2px solid var(--ink);
    overflow: hidden;
    background: var(--chip);
  }
  .grp {
    display: grid;
    place-items: center;
    height: 100%;
    font: 18px var(--font-disp);
    color: var(--bone);
    background: var(--ink);
  }
  .chat-title {
    display: flex;
    align-items: baseline;
    gap: 6px;
    font: 600 14px var(--font-ui);
    margin-bottom: 7px;
    min-width: 0;
  }
  .chat-title > span {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .who {
    font: 10.5px var(--font-mono);
    color: var(--dim);
    margin-left: 6px;
  }
  .link {
    border: 0;
    background: none;
    padding: 0;
    font: inherit;
    text-decoration: underline;
    cursor: pointer;
    color: inherit;
  }
  .chat-bar {
    height: 10px;
    background: var(--hair);
  }
  .chat-bar i {
    display: block;
    height: 100%;
  }
  .chat-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .chat-meta b {
    font: 17px var(--font-disp);
    font-weight: 400;
  }
  .chat-meta span {
    font: 9px var(--font-mono);
    text-transform: uppercase;
    color: var(--dim);
  }
  .open {
    padding: 7px 10px !important;
  }

  @media (max-width: 1100px) {
    .rhythm,
    .voices {
      grid-template-columns: 1fr;
    }
    .clock {
      max-width: 400px;
      margin: 0 auto;
    }
    .player {
      grid-template-columns: 1fr 1fr;
    }
    .lengths {
      grid-column: 1 / -1;
      order: 3;
    }
    .hero h2 {
      font-size: 56px;
    }
    .hero-number {
      font-size: 130px;
      top: -14px;
    }
  }
</style>
