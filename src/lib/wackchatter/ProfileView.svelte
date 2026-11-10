<script lang="ts">
  import Chart from "$lib/Chart.svelte";
  import Portrait from "./Portrait.svelte";
  import Section from "./Section.svelte";
  import TotalsStrip from "./TotalsStrip.svelte";
  import type { Stats } from "./types";
  import type { Auras } from "./auras.svelte";
  import { fmtTokens } from "$lib/format";
  import { foldActivity, foldDays, milestones } from "./buckets";
  import { journey, share, stackedLanes } from "./charts";
  import { count, date } from "./labels";

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
    accent: { fill: string; ink: string };
    navigate: (patch: Record<string, string | null>) => void;
    open: (chat?: string | null, char?: string | null) => void;
  } = $props();

  const aura = (id: string) => auras.of(library, id, stats.colors.characters);
  const own = $derived(
    character ? stats.cast_activity.filter((a) => a[0] === character) : stats.cast_activity,
  );
  const days = $derived(foldDays(own.map(([, t, n]) => [t, n] as [number, number])));
  const marks = $derived(milestones(days));
  const replies = $derived(own.reduce((t, a) => t + a[2], 0));
  const span = $derived(
    stats.first_chat && stats.last_chat
      ? Math.max(1, Math.round((stats.last_chat - stats.first_chat) / 86400000))
      : 0,
  );

  const speakers = $derived(
    stats.cast
      .filter((c) => c.replies > 0)
      .map((c) => ({ name: c.name, value: c.replies, color: aura(c.character_id).fill })),
  );
  const weekly = $derived(foldActivity(stats.cast_activity, "week"));
  const speakerLanes = $derived(
    stats.cast
      .filter((c) => weekly.series.has(c.character_id))
      .map((c) => ({
        id: c.character_id,
        name: c.name,
        color: aura(c.character_id).fill,
        values: weekly.series.get(c.character_id)!,
      })),
  );
</script>

<div class="profile up">
  <button class="wc-btn back" onclick={() => navigate({ character: null, group: null })}>← Library</button>
  <div class="head">
    {#if character}
      <div class="portrait"><Portrait {library} id={character} name={stats.name} tint={accent.fill} letter={90} /></div>
    {:else}
      <div class="ensemble">
        {#each stats.cast.slice(0, 3) as c (c.character_id)}
          <div class="mini" style="background:{aura(c.character_id).fill}">
            <Portrait {library} id={c.character_id} name={c.name} tint={aura(c.character_id).fill} letter={28} />
          </div>
        {/each}
        <div class="mini rest">
          <b>{count(replies)}</b><span>replies</span>
        </div>
      </div>
    {/if}
    <div class="who">
      <p class="eyebrow">{group ? "Group conversation" : "Character"}</p>
      <h1>{stats.name}</h1>
      <div class="facts">
        <div><b>{date(stats.first_chat)}</b><span>Together since</span></div>
        <div><b>{date(stats.last_chat)}</b><span>Last spoke</span></div>
        <div><b>{count(span)}d</b><span>Span</span></div>
        <div><b>{count(replies)}</b><span>{group ? "Replies in this group" : "Replies"}</span></div>
      </div>
    </div>
    <button class="wc-btn go" onclick={() => open(group, character)}>Open in WackChatter ↗</button>
  </div>
  {#if character}<p class="wc-note">
      Figures below describe direct chats. Group participation is shown separately.
    </p>{/if}
</div>

<TotalsStrip {stats} />

<Section index="01" kicker={group ? "The ensemble" : "Your story"} title={group ? "Who carries the scene" : "The road so far"} note={group ? "Replies per speaker, all time and by local week" : "Cumulative replies by local day · Pins mark the first reply and every order of magnitude"}>
  {#if group}
    <div class="split">
      {#if speakers.length}<Chart option={share(speakers)} height={300} />{/if}
      {#if speakerLanes.length && weekly.periods.length > 1}<Chart option={stackedLanes(weekly.periods, speakerLanes)} height={300} />{/if}
    </div>
  {:else if days.length}
    <Chart option={journey(days, marks, accent.fill)} height={300} />
    <div class="milestones">
      {#each marks as m (m.count)}
        <div><b>{m.count === 1 ? "First reply" : `${count(m.count)} replies`}</b><span>{date(m.day)}</span></div>
      {/each}
    </div>
  {:else}<p class="wc-note">No dated replies yet.</p>{/if}
  {#snippet table()}
    {#if group}
      <div class="wc-scroll">
        <table class="wc-table">
          <thead><tr><th>Speaker</th><th class="num">Replies</th><th class="num">Measured tokens</th><th class="num">Estimated tokens</th></tr></thead>
          <tbody>
            {#each stats.cast as c (c.character_id)}<tr>
                <td><i class="wc-swatch" style="background:{aura(c.character_id).fill}"></i>
                  <button class="link" onclick={() => navigate({ character: c.character_id, group: null })}>{c.name}</button></td>
                <td class="num">{count(c.replies)}</td><td class="num">{fmtTokens(c.measured_tokens)}</td><td class="num">{fmtTokens(c.estimated_tokens)}</td>
              </tr>{/each}
          </tbody>
        </table>
      </div>
    {/if}
  {/snippet}
</Section>

<style>
  .profile {
    margin: 0 0 24px;
  }
  .back {
    margin-bottom: 18px;
  }
  .head {
    display: flex;
    align-items: stretch;
    border: 2px solid var(--ink);
    background: var(--aura-wash);
    position: relative;
  }
  .portrait {
    width: 200px;
    height: 250px;
    flex: none;
    border-right: 2px solid var(--ink);
  }
  .ensemble {
    width: 220px;
    flex: none;
    display: grid;
    grid-template-columns: 1fr 1fr;
    border-right: 2px solid var(--ink);
  }
  .mini {
    min-height: 110px;
    border: 1px solid var(--ink);
    overflow: hidden;
  }
  .mini.rest {
    background: var(--ink);
    color: var(--bone);
    display: flex;
    flex-direction: column;
    justify-content: center;
    align-items: center;
    gap: 4px;
  }
  .mini.rest b {
    font: 34px/1 var(--font-disp);
  }
  .mini.rest span {
    font: 9px var(--font-mono);
    text-transform: uppercase;
    letter-spacing: 1px;
  }
  .who {
    padding: 24px 28px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    border-left: 14px solid var(--aura-ink);
  }
  .eyebrow {
    font: 11px var(--font-mono);
    letter-spacing: 1.5px;
    text-transform: uppercase;
    margin: 0;
    color: var(--aura-ink);
  }
  h1 {
    font: 72px/0.9 var(--font-disp);
    margin: 0;
    text-transform: uppercase;
  }
  .facts {
    display: flex;
    gap: 30px;
    flex-wrap: wrap;
    margin-top: auto;
  }
  .facts div {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .facts b {
    font: 22px var(--font-disp);
    font-weight: 400;
  }
  .facts span {
    font: 9.5px var(--font-mono);
    text-transform: uppercase;
    color: var(--aura-ink);
  }
  .go {
    position: absolute;
    top: 18px;
    right: 18px;
    background: var(--bone) !important;
  }
  .go:hover {
    background: var(--ink) !important;
  }
  .split {
    display: grid;
    grid-template-columns: 380px minmax(0, 1fr);
    gap: 26px;
  }
  .milestones {
    display: flex;
    gap: 2px;
    background: var(--ink);
    border: 2px solid var(--ink);
    margin-top: 12px;
  }
  .milestones div {
    flex: 1;
    background: var(--bone);
    padding: 10px 14px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .milestones div:first-child {
    background: var(--aura);
  }
  .milestones b {
    font: 17px var(--font-disp);
    font-weight: 400;
    text-transform: uppercase;
  }
  .milestones span {
    font: 10px var(--font-mono);
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
  @media (max-width: 1100px) {
    h1 {
      font-size: 52px;
    }
    .split {
      grid-template-columns: 1fr;
    }
    .go {
      position: static;
      align-self: start;
      margin: 18px;
    }
  }
</style>
