<script lang="ts">
  import { onMount } from "svelte";
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import AnimatedNumber from "$lib/AnimatedNumber.svelte";
  import Chart from "$lib/Chart.svelte";
  import Portrait from "$lib/wackchatter/Portrait.svelte";
  import { wackchatterApi as api } from "$lib/api";
  import type {
    Library,
    Stats,
    Usage,
    Breakdown,
    Call,
  } from "$lib/wackchatter/types";
  import {
    foldDays,
    foldHourOfDay,
    foldWeekday,
    recentDays,
    streaks,
    busiestDay,
  } from "$lib/wackchatter/buckets";
  import { fmtTokens, fmtCost, MODEL_PALETTE } from "$lib/format";
  import {
    TOOLTIP,
    MONO,
    INK,
    HAIR,
    ANIM,
    donutSeries,
    dateTick,
  } from "$lib/chartTheme";
  import { readPref, writePref } from "$lib/prefs";
  import type { EChartsOption } from "echarts";

  let libraries = $state<Library[]>([]),
    stats = $state<Stats | null>(null),
    usage = $state<Usage | null>(null);
  let library = $state(""),
    loading = $state(true),
    error = $state(""),
    actionError = $state("");
  let days = $state(
    readPref(
      "tt.wackchatter.days",
      90,
      (v) => typeof v === "number" && [0, 7, 30, 90].includes(v),
    ),
  );
  let allUsage = $state(false),
    expanded = $state(""),
    calls = $state<Call[]>([]),
    callsLoading = $state(false),
    libsReady = $state(false);
  let seq = 0,
    callSeq = 0;
  // The stats the page shows are keyed on (library, character, group); a load
  // for the same view — a tt-sync tick or a range flip — refreshes the data
  // in place instead of unmounting the whole page behind the splash screen.
  let statsKey = "";
  const tab = $derived(
    $page.url.searchParams.get("tab") === "usage" ? "usage" : "library",
  );
  const character = $derived($page.url.searchParams.get("character"));
  const group = $derived($page.url.searchParams.get("group"));
  const calendar = $derived(foldDays(stats?.hours ?? []));
  const streak = $derived(streaks(calendar));
  const busiest = $derived(busiestDay(calendar));
  const top = $derived(stats?.cast[0]);
  const currentLibrary = $derived(libraries.find((l) => l.id === library));
  const labels: Record<string, string> = {
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
  const count = (n: number) => Math.round(n).toLocaleString();
  const date = (n: number | null) =>
    n === null
      ? "—"
      : new Date(n).toLocaleDateString(undefined, {
          day: "numeric",
          month: "short",
          year: "numeric",
        });
  const duration = (n: number | null) =>
    n === null
      ? "—"
      : n < 1000
        ? `${Math.round(n)}ms`
        : `${(n / 1000).toFixed(1)}s`;
  const percent = (n: number, d: number) =>
    d ? `${((n / d) * 100).toFixed(1)}%` : "—";
  function navigate(patch: Record<string, string | null>) {
    const q = new URLSearchParams($page.url.search);
    if (library) q.set("library", library);
    for (const [k, v] of Object.entries(patch)) {
      if (v === null) q.delete(k);
      else q.set(k, v);
    }
    void goto("/wackchatter?" + q);
  }
  async function refreshLibraries() {
    try {
      libraries = await api.libraries();
      if (!library)
        library =
          $page.url.searchParams.get("library") ||
          readPref(
            "tt.wackchatter.library",
            "",
            (v) => typeof v === "string" && libraries.some((l) => l.id === v),
          ) ||
          libraries.find((l) => l.current)?.id ||
          libraries[0]?.id ||
          "";
    } catch (e) {
      error = String(e);
      loading = false;
    } finally {
      libsReady = true;
    }
  }
  async function load(
    lib: string,
    char: string | null,
    grp: string | null,
    range: number,
    all: boolean,
  ) {
    const key = JSON.stringify([lib, char, grp]);
    const quiet = key === statsKey;
    const request = ++seq;
    // until the first library sweep lands we don't know whether `lib` should
    // be empty — loading with "" would flash the "no library" empty state
    if (!lib && !libsReady) return;
    error = "";
    if (!quiet) {
      statsKey = key;
      loading = true;
      stats = null;
      usage = null;
      expanded = "";
      calls = [];
      callsLoading = false;
      callSeq++;
    }
    const results = await Promise.allSettled([
      lib ? api.stats(lib, char, grp) : Promise.resolve(null),
      api.usage(all ? null : lib || null, range),
      // an expanded conversation keeps its call list live across syncs
      expanded
        ? api.usage(all ? null : lib || null, range, expanded)
        : Promise.resolve(null),
    ]);
    if (request !== seq) return;
    if (results[0].status === "fulfilled") stats = results[0].value;
    else error = String(results[0].reason);
    if (results[1].status === "fulfilled") usage = results[1].value;
    else error += (error ? " · " : "") + String(results[1].reason);
    if (results[2].status === "fulfilled" && results[2].value)
      calls = results[2].value.calls;
    loading = false;
  }
  $effect(() => {
    const requested = $page.url.searchParams.get("library");
    if (requested && requested !== library) library = requested;
  });
  $effect(() => {
    void load(library, character, group, days, allUsage);
  });
  $effect(() => {
    writePref("tt.wackchatter.days", days);
    if (library) writePref("tt.wackchatter.library", library);
  });
  onMount(() => {
    void refreshLibraries();
    const sync = () => {
      void refreshLibraries();
      void load(library, character, group, days, allUsage);
    };
    window.addEventListener("tt-sync", sync);
    return () => {
      ++seq;
      window.removeEventListener("tt-sync", sync);
    };
  });
  async function open(chat: string | null = null, char: string | null = null) {
    actionError = "";
    try {
      await api.open(library, chat, char);
    } catch (e) {
      actionError = String(e);
    }
  }
  async function expand(id: string) {
    if (expanded === id) {
      expanded = "";
      calls = [];
      callSeq++;
      return;
    }
    expanded = id;
    calls = [];
    callsLoading = true;
    // its own counter: a tt-sync bumps seq, which used to strand callsLoading
    const request = ++callSeq;
    try {
      const d = await api.usage(allUsage ? null : library || null, days, id);
      if (expanded === id && callSeq === request) calls = d.calls;
    } catch (e) {
      if (expanded === id && callSeq === request) actionError = String(e);
    } finally {
      if (expanded === id && callSeq === request) callsLoading = false;
    }
  }
  function bars(
    labels: string[],
    values: number[],
    name: string,
    color = "#c8e600",
    xTick?: (v: string) => string,
  ): EChartsOption {
    return {
      ...ANIM,
      tooltip: { ...TOOLTIP, trigger: "axis" },
      grid: { left: 52, right: 12, top: 15, bottom: 35 },
      xAxis: {
        type: "category",
        data: labels,
        axisLabel: {
          fontFamily: MONO,
          fontSize: 10,
          color: INK,
          ...(xTick ? { formatter: xTick } : {}),
        },
        axisLine: { lineStyle: { color: INK } },
      },
      yAxis: {
        type: "value",
        splitLine: { lineStyle: { color: HAIR } },
        axisLabel: {
          color: INK,
          fontFamily: MONO,
          fontSize: 10,
          formatter: (v: number) => fmtTokens(v),
        },
      },
      series: [{ name, type: "bar", data: values, itemStyle: { color } }],
    };
  }
  const dayChart = $derived.by(() => {
    const d = recentDays(calendar, 84);
    return bars(
      d.map((x) =>
        new Date(x.day).toLocaleDateString(undefined, {
          day: "numeric",
          month: "short",
        }),
      ),
      d.map((x) => x.count),
      "Messages",
    );
  });
  const dailyUsage = $derived.by(() => {
    const d = [...(usage?.daily ?? [])].sort((a, b) =>
      a.name.localeCompare(b.name),
    );
    const option = bars(
      d.map((x) => x.name),
      d.map((x) => x.measured_tokens),
      "Measured tokens",
      "#c8e600",
      dateTick,
    );
    option.series = [
      {
        name: "Measured tokens",
        type: "bar",
        data: d.map((x) => x.measured_tokens),
        itemStyle: { color: "#c8e600" },
      },
      {
        name: "Estimated completion tokens",
        type: "bar",
        data: d.map((x) => x.estimated_tokens),
        itemStyle: { color: "#7c5cff" },
      },
    ];
    return option;
  });
  const modelChart = $derived.by<EChartsOption | undefined>(() => {
    const models = stats?.models ?? [];
    if (!models.length) return undefined;
    return {
      backgroundColor: "transparent",
      ...ANIM,
      tooltip: {
        ...TOOLTIP,
        formatter: (p: any) =>
          `${p.name}<br/>${count(Number(p.value ?? 0))} swipes (${p.percent}%)`,
      },
      // shared donut shape: labels stay off, the table next to it names the
      // slices — color swatches match by rank
      series: [
        donutSeries(
          models.map((m, i) => ({
            name: `${m.model} · ${m.provider}`,
            value: m.swipes,
            itemStyle: { color: MODEL_PALETTE[i % MODEL_PALETTE.length] },
          })),
        ),
      ],
    };
  });
  const conversationName = (id: string) => {
    const chat =
      stats?.chats.find((c) => c.id === id) ??
      stats?.groups.find((c) => c.id === id);
    if (!chat) return id === "unknown" ? "Unknown conversation" : id;
    // most chats are titled "+ New Chat" — the character they belong to is
    // what actually tells the rows apart
    const who = chat.character_id
      ? stats?.cast.find((c) => c.character_id === chat.character_id)?.name
      : undefined;
    return who ? `${chat.title} · ${who}` : chat.title;
  };
</script>

<svelte:head><title>WackChatter · TokenTrail</title></svelte:head>
<div class="wc-page up">
  <header class="wc-header">
    <div>
      <p class="eyebrow">The Wack ecosystem · Your stories, in numbers</p>
      <h1>WackChatter<span>STATS</span></h1>
      <p>Characters, conversations, and every generation behind them.</p>
    </div>
    <div class="tools">
      {#if libraries.length > 1}<label
          >Library<select
            value={library}
            onchange={(e) => {
              library = e.currentTarget.value;
              navigate({ library, character: null, group: null });
            }}
            >{#each libraries as l}<option value={l.id}
                >{l.name}{l.current ? " · current" : ""}{!l.available
                  ? " · unavailable"
                  : ""}</option
              >{/each}</select
          ></label
        >{:else if currentLibrary}<span class="library-tag"
          >{currentLibrary.name}</span
        >{/if}
      <button
        onclick={() => {
          void refreshLibraries();
          void load(library, character, group, days, allUsage);
        }}
        disabled={loading}>↻ Refresh</button
      >
      <button onclick={() => open()} disabled={!library}
        >Open WackChatter ↗</button
      >
    </div>
  </header>
  <div class="tabs">
    <button
      class:active={tab === "library"}
      onclick={() => navigate({ tab: null })}
      >Library <small>ALL TIME</small></button
    ><button
      class:active={tab === "usage"}
      onclick={() => navigate({ tab: "usage", character: null, group: null })}
      >Usage</button
    ><span>Local data · Read-only</span>
  </div>
  {#if actionError}<p class="error" role="alert">{actionError}</p>{/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if loading}<p class="empty" role="status">Reading your library…</p>
  {:else if tab === "library"}
    {#if !stats && error}<div class="empty">
        <h2>Library unavailable</h2>
        <p>
          Reconnect the library or start WackChatter with it, then refresh.
          Recorded usage remains available in the Usage tab.
        </p>
      </div>
    {:else if !stats}<div class="empty">
        <h2>Your stories start here</h2>
        <p>
          Start WackChatter once to discover its library. Stats will then work
          even when WackChatter is closed.
        </p>
        <a
          href="https://github.com/Intelios/wackchatter"
          target="_blank"
          rel="noreferrer">Get WackChatter ↗</a
        ><button
          onclick={() =>
            navigate({ tab: "usage", character: null, group: null })}
          >View recorded usage</button
        >
      </div>
    {:else}
      {#if character || group}<div class="detail-heading">
          <button onclick={() => navigate({ character: null, group: null })}
            >← Library</button
          >{#if character}<div class="detail-portrait">
              <Portrait {library} id={character} name={stats.name} />
            </div>{/if}
          <div>
            <p class="eyebrow">{group ? "GROUP CONVERSATION" : "CHARACTER"}</p>
            <h2>{stats.name}</h2>
            <p>
              Together since {date(stats.first_chat)} · Last spoke {date(
                stats.last_chat,
              )}
            </p>
          </div>
          <button onclick={() => open(group, character)}
            >Open in WackChatter ↗</button
          >
        </div>{/if}
      {#if character}<p class="note">
          These figures describe direct chats. Group participation is shown
          separately below.
        </p>{/if}
      <div class="tiles">
        {#each [["Chats", stats.totals.chats], ["Messages", stats.totals.messages], ["Your messages", stats.totals.user_messages], ["Replies", stats.totals.replies], ["Generated swipes", stats.totals.swipes]] as item}
          <div class="tile">
            <span>{item[0]}</span><strong
              ><AnimatedNumber value={Number(item[1])} format={count} /></strong
            >
          </div>
        {/each}
        <div class="tile accent">
          <span>Time in scene · estimated</span><strong
            >{Math.floor(stats.totals.active_minutes / 60)}<small>h</small>
            {stats.totals.active_minutes % 60}<small>m</small></strong
          >
        </div>
      </div>
      <div class="token-strip">
        <div>
          <span>Measured completion tokens</span><b
            >{fmtTokens(stats.totals.measured_tokens)}</b
          >
        </div>
        <div>
          <span>Estimated completion tokens</span><b
            >{fmtTokens(stats.totals.estimated_tokens)}</b
          >
        </div>
        <p>
          Completion counts from saved replies. Estimates are never priced. Full
          generation accounting lives in Usage.
        </p>
      </div>
      {#if !character && !group && top}
        <button
          class="hero"
          onclick={() => navigate({ character: top.character_id, group: null })}
          ><div class="hero-portrait">
            <Portrait {library} id={top.character_id} name={top.name} />
          </div>
          <div>
            <p class="eyebrow">Most played · By recorded replies</p>
            <h2>{top.name}</h2>
            <p>
              {count(top.replies)} replies · {count(top.chats)} conversations
            </p>
            <span>Explore your history together →</span>
          </div>
          <b class="hero-number">01</b></button
        >
      {/if}
      {#if !character && stats.cast.length}<section>
          <div class="section-title">
            <span>01 / THE CAST</span>
            <h2>Who you spend it with</h2>
          </div>
          <div class="cast-grid">
            {#each stats.cast as c, i (c.character_id)}<button
                class="cast-card"
                onclick={() =>
                  navigate({ character: c.character_id, group: null })}
                ><div class="cast-image">
                  <Portrait {library} id={c.character_id} name={c.name} /><span
                    >{String(i + 1).padStart(2, "0")}</span
                  >
                </div>
                <h3>{c.name}</h3>
                <p>{count(c.replies)} replies · {count(c.chats)} chats</p>
                <small
                  >{fmtTokens(c.measured_tokens)} measured · {fmtTokens(
                    c.estimated_tokens,
                  )} estimated tokens</small
                ></button
              >{/each}
          </div>
        </section>{/if}
      <section>
        <div class="section-title">
          <span>02 / THE MODELS</span>
          <h2>What is doing the writing</h2>
        </div>
        <div class="model-layout">
          <div class="model-donut">
            {#if modelChart}<Chart option={modelChart} height={280} />{/if}
          </div>
          <div class="scroll cap">
            <table>
              <thead
                ><tr
                  ><th>Model / provider</th><th>Swipes</th><th
                    >Measured tokens</th
                  ><th>Estimated tokens</th><th>Latency</th><th
                    >Measured tok/s</th
                  ><th>Est. tok/s</th></tr
                ></thead
              ><tbody
                >{#each stats.models as m, i (m.model + m.provider)}<tr
                    ><td
                      ><i
                        class="swatch"
                        style="background:{MODEL_PALETTE[
                          i % MODEL_PALETTE.length
                        ]}"
                      ></i
                      >{m.model}<small>{m.provider}</small></td
                    ><td
                      >{count(m.swipes)}</td
                    ><td>{fmtTokens(m.measured_tokens)}</td><td
                      >{fmtTokens(m.estimated_tokens)}</td
                    ><td
                      >{duration(
                        m.timed_swipes ? m.total_ms / m.timed_swipes : null,
                      )}</td
                    ><td
                      >{m.measured_ms > 0
                        ? (
                            (m.measured_timed_tokens * 1000) /
                            m.measured_ms
                          ).toFixed(1)
                        : "—"}</td
                    ><td
                      >{m.estimated_ms > 0
                        ? (
                            (m.estimated_timed_tokens * 1000) /
                            m.estimated_ms
                          ).toFixed(1)
                        : "—"}</td
                    ></tr
                  >{/each}</tbody
              >
            </table>
          </div>
        </div>
      </section>
      <section>
        <div class="section-title">
          <span>03 / PERSONAS</span>
          <h2>Who you show up as</h2>
        </div>
        <div class="personas">
          {#each stats.personas as p (p.persona_id ?? p.name)}<div class="persona">
              <div class="avatar">
                <Portrait
                  {library}
                  id={p.persona_id ?? ""}
                  name={p.name}
                  persona
                />
              </div>
              <div>
                <h3>{p.name}</h3>
                <p>{count(p.messages)} messages · {count(p.chats)} chats</p>
              </div>
            </div>{/each}{#if !stats.personas.length}<p>
              No persona activity yet.
            </p>{/if}
        </div>
      </section>
      <section>
        <div class="section-title">
          <span>04 / YOUR RHYTHM</span>
          <h2>When you play</h2>
        </div>
        <div class="facts">
          <div><b>{calendar.length}</b><span>Active days</span></div>
          <div><b>{streak.current}d</b><span>Current streak</span></div>
          <div><b>{streak.longest}d</b><span>Longest streak</span></div>
          <div>
            <b>{busiest ? date(busiest.day) : "—"}</b><span
              >Busiest day{busiest
                ? ` · ${count(busiest.count)} messages`
                : ""}</span
            >
          </div>
        </div>
        <p class="note">Your local calendar · Last 84 days</p>
        <Chart option={dayChart} height={240} />
        <div class="two-charts">
          <div>
            <h3>Hour of day</h3>
            <Chart
              option={bars(
                Array.from({ length: 24 }, (_, i) => `${i}:00`),
                foldHourOfDay(stats.hours),
                "Messages",
              )}
              height={210}
            />
          </div>
          <div>
            <h3>Day of week</h3>
            <Chart
              option={bars(
                ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
                foldWeekday(stats.hours),
                "Messages",
              )}
              height={210}
            />
          </div>
        </div>
      </section>
      <section>
        <div class="section-title">
          <span>05 / HABITS</span>
          <h2>How you play</h2>
        </div>
        <div class="facts habit-grid">
          {#each [["Rerolls", count(stats.habits.rerolls)], ["Replies rerolled", percent(stats.habits.rerolled_replies, stats.habits.eligible_replies)], ["Average user message", `${count(stats.habits.avg_user_chars)} chars`], ["Average reply", `${count(stats.habits.avg_reply_chars)} chars`], ["Longest reply", `${count(stats.habits.longest_reply_chars)} chars`], ["Reasoning swipes", `${count(stats.habits.reasoning_swipes)} · ${percent(stats.habits.reasoning_swipes, stats.totals.swipes)}`], ["Timed swipes", count(stats.habits.timed_swipes)], ["Branched chats", count(stats.habits.branched_chats)], ["Chats in trash", count(stats.habits.deleted_chats)], ["Co-Creator sessions", count(stats.co_creator_sessions)], ["Longest conversation", stats.chats[0] ? `${count(stats.chats[0].messages)} messages` : "—"]] as [label, value]}<div
            >
              <b>{value}</b><span>{label}</span>
            </div>{/each}
        </div>
        {#if stats.chats[0]}<p class="note">
            Longest conversation: {stats.chats[0].title}
          </p>{/if}
      </section>
      <section>
        <div class="section-title">
          <span>06 / CONVERSATIONS</span>
          <h2>
            {group
              ? "This group"
              : character
                ? "The long conversations"
                : "Your conversations"}
          </h2>
        </div>
        <div class="scroll cap">
          <table>
            <thead
              ><tr
                ><th>Conversation</th><th>Messages</th><th>Replies</th><th
                  >Measured tokens</th
                ><th>Estimated tokens</th><th>Last spoke</th><th></th></tr
              ></thead
            ><tbody
              >{#each stats.chats as c (c.id)}<tr
                  ><td
                    >{#if c.kind === "group"}<button
                        class="text-button"
                        onclick={() =>
                          navigate({ group: c.id, character: null })}
                        >{c.title} <small>GROUP</small></button
                      >{:else}{c.title}{/if}</td
                  ><td>{count(c.messages)}</td><td>{count(c.replies)}</td><td
                    >{fmtTokens(c.measured_tokens)}</td
                  ><td>{fmtTokens(c.estimated_tokens)}</td><td
                    >{date(c.modified)}</td
                  ><td><button onclick={() => open(c.id)}>Open ↗</button></td
                  ></tr
                >{/each}</tbody
            >
          </table>
        </div>
        {#if !stats.chats.length}<p>No direct conversations yet.</p>{/if}
      </section>
      {#if character && stats.groups.length}<section>
          <div class="section-title">
            <span>07 / GROUP PARTICIPATION</span>
            <h2>In the ensemble</h2>
          </div>
          <p class="note">
            Replies and tokens belong to {stats.name}. Shared user messages and
            time stay with the group.
          </p>
          <div class="scroll">
            <table>
              <thead
                ><tr
                  ><th>Group</th><th>Character replies</th><th
                    >Measured tokens</th
                  ><th>Estimated tokens</th><th></th></tr
                ></thead
              ><tbody
                >{#each stats.groups as g (g.id)}<tr
                    ><td
                      ><button
                        class="text-button"
                        onclick={() =>
                          navigate({ group: g.id, character: null })}
                        >{g.title}</button
                      ></td
                    ><td>{count(g.replies)}</td><td
                      >{fmtTokens(g.measured_tokens)}</td
                    ><td>{fmtTokens(g.estimated_tokens)}</td><td
                      ><button onclick={() => open(g.id)}>Open ↗</button></td
                    ></tr
                  >{/each}</tbody
              >
            </table>
          </div>
        </section>{/if}
    {/if}
  {:else if usage}
    <div class="usage-controls">
      <p>Generation history survives changes to your library.</p>
      <label
        ><input type="checkbox" bind:checked={allUsage} /> All libraries, including
        unattributed history</label
      >
      <div class="range">
        {#each [[7, "7D"], [30, "30D"], [90, "90D"], [0, "ALL"]] as [n, label]}<button
            class:active={days === n}
            onclick={() => (days = Number(n))}>{label}</button
          >{/each}
      </div>
    </div>
    <div class="tiles usage-tiles">
      <div class="tile">
        <span>Generations</span><strong>{count(usage.totals.events)}</strong>
      </div>
      <div class="tile accent">
        <span>Measured tokens</span><strong
          >{fmtTokens(
            usage.totals.input_tokens +
              usage.totals.output_tokens +
              usage.totals.cache_read_tokens +
              usage.totals.cache_write_tokens,
          )}</strong
        >
      </div>
      <div class="tile">
        <span>Estimated completion tokens</span><strong
          >{fmtTokens(usage.totals.estimated_tokens)}</strong
        >
      </div>
      <div class="tile">
        <span>API-equivalent cost estimate</span><strong
          >{usage.totals.cost_usd === null
            ? "—"
            : fmtCost(usage.totals.cost_usd)}</strong
        >
      </div>
    </div>
    <p class="note">
      List-price estimate, not actual subscription spend. {count(
        usage.totals.priced_events,
      )} of {count(usage.totals.measured_events)} measured generations priced. Estimated
      tokens are never priced.
    </p>
    <div class="facts">
      {#each [["Input", fmtTokens(usage.totals.input_tokens)], ["Output", fmtTokens(usage.totals.output_tokens)], ["Cache read", fmtTokens(usage.totals.cache_read_tokens)], ["Cache write", fmtTokens(usage.totals.cache_write_tokens)], ["Average duration", duration(usage.totals.avg_duration_ms)], ["First token", duration(usage.totals.avg_ttft_ms)]] as [label, value]}<div
        >
          <b>{value}</b><span>{label}</span>
        </div>{/each}
    </div>
    {#if usage.totals.events === 0}<div class="empty">
        <h2>No recorded generations in this range</h2>
        <p>
          Enable “Record usage for TokenTrail” in WackChatter, or choose a wider
          range. Existing transcript usage is imported automatically.
        </p>
      </div>{/if}
    <section>
      <div class="section-title">
        <span>01 / GENERATIONS</span>
        <h2>The work behind your stories</h2>
      </div>
      <p class="note">
        Daily usage · UTC · Measured and estimated counts shown separately
      </p>
      <Chart option={dailyUsage} height={280} />
    </section>
    <div class="breakdowns">
      {#each [["Features", usage.features], ["Models", usage.models], ["Providers", usage.providers], ["Characters", usage.characters], ["Outcomes", usage.outcomes]] as section}
        {@const title = section[0] as string}{@const data =
          section[1] as Breakdown[]}
        <section>
          <h2>{title}</h2>
          <div class="scroll cap">
            <table>
              <thead
                ><tr
                  ><th>Name</th><th>Calls</th><th>Measured</th><th>Estimated</th
                  ><th>API-equiv.</th></tr
                ></thead
              ><tbody
                >{#each data as r (r.name)}<tr
                    ><td
                      >{title === "Features"
                        ? (labels[r.name] ?? r.name)
                        : title === "Characters"
                          ? (stats?.cast.find((c) => c.character_id === r.name)
                              ?.name ?? r.name)
                          : r.name}</td
                    ><td>{count(r.events)}</td><td
                      >{fmtTokens(r.measured_tokens)}</td
                    ><td>{fmtTokens(r.estimated_tokens)}</td><td
                      >{r.cost_usd === null ? "—" : fmtCost(r.cost_usd)}</td
                    ></tr
                  >{/each}</tbody
              >
            </table>
          </div>
        </section>
      {/each}
    </div>
    <section>
      <div class="section-title">
        <span>02 / CONVERSATIONS</span>
        <h2>Follow the generations</h2>
      </div>
      <p class="note">
        Expand a conversation to inspect feature, outcome, timing, and token
        counts.
      </p>
      <div class="chat-list">
        {#each usage.conversations as c (c.name)}<div class="usage-chat">
          <button
            onclick={() => expand(c.name)}
            aria-expanded={expanded === c.name}
            ><span
              >{expanded === c.name ? "−" : "+"}
              {conversationName(c.name)}</span
            ><span
              >{count(c.events)} calls · {fmtTokens(c.measured_tokens)} measured ·
              {fmtTokens(c.estimated_tokens)} estimated</span
            ></button
          >{#if expanded === c.name}{#if callsLoading}<p>
                Loading generations…
              </p>{:else}<div class="scroll">
                <table>
                  <thead
                    ><tr
                      ><th>Time</th><th>Feature</th><th>Model</th><th
                        >Outcome</th
                      ><th>Measured</th><th>Estimated</th><th
                        >Duration / first token</th
                      ></tr
                    ></thead
                  ><tbody
                    >{#each calls as c (c.id)}<tr
                        ><td>{new Date(c.ts).toLocaleString()}</td><td
                          >{labels[c.purpose] ?? c.purpose}</td
                        ><td>{c.model}</td><td>{c.outcome}</td><td
                          >{fmtTokens(c.measured_tokens)}</td
                        ><td>{fmtTokens(c.estimated_tokens)}</td><td
                          >{duration(c.duration_ms)} / {duration(c.ttft_ms)}</td
                        ></tr
                      >{/each}</tbody
                  >
                </table>
              </div>{/if}{/if}
          </div>{/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .wc-page {
    /* edge-to-edge like the rest of the app — no centered column */
    padding: 36px clamp(22px, 1.8vw, 40px) 70px;
  }
  .wc-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 22px;
    margin-bottom: 30px;
  }
  .eyebrow,
  .section-title > span {
    font: 11px var(--font-mono);
    letter-spacing: 1.5px;
    text-transform: uppercase;
  }
  .eyebrow {
    margin: 0 0 10px;
  }
  h1 {
    font: 56px/1 var(--font-disp);
    margin: 0;
    letter-spacing: -1px;
  }
  h1 span {
    font: 13px var(--font-mono);
    letter-spacing: 2px;
    vertical-align: middle;
    margin-left: 16px;
    padding: 7px 10px;
    background: var(--acd);
    border: 2px solid var(--ink);
  }
  h2 {
    font: 30px var(--font-disp);
    margin: 8px 0 16px;
  }
  h3 {
    font: 600 16px var(--font-ui);
    margin: 10px 0 5px;
  }
  p {
    line-height: 1.6;
  }
  .wc-header p:last-child {
    color: var(--dim);
    margin-bottom: 0;
  }
  .tools {
    display: flex;
    align-items: end;
    justify-content: end;
    gap: 8px;
    flex-wrap: wrap;
  }
  .tools label {
    display: grid;
    gap: 5px;
    font: 11px var(--font-mono);
  }
  button,
  select {
    background: transparent;
    border: 2px solid var(--ink);
    padding: 9px 12px;
    color: var(--ink);
    font: 600 12px var(--font-ui);
    cursor: pointer;
    border-radius: 0;
  }
  button:hover {
    background: var(--ink);
    color: var(--bone);
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  button:focus-visible,
  a:focus-visible {
    outline: 3px solid var(--vio);
    outline-offset: 3px;
  }
  .library-tag {
    font: 12px var(--font-mono);
    padding: 10px;
  }
  .tabs {
    display: flex;
    border-bottom: 2px solid var(--ink);
    gap: 0;
    margin-bottom: 28px;
  }
  .tabs button {
    border-bottom: 0;
    padding: 14px 25px;
  }
  .tabs small {
    margin-left: 12px;
    font: 9px var(--font-mono);
  }
  .tabs > span {
    margin-left: auto;
    align-self: center;
    font: 10px var(--font-mono);
    color: var(--dim);
  }
  .active {
    background: var(--ink);
    color: var(--bone);
  }
  .tiles {
    display: grid;
    grid-template-columns: repeat(6, minmax(0, 1fr));
    border: 2px solid var(--ink);
    gap: 2px;
    background: var(--ink);
  }
  .tile {
    background: var(--bone);
    padding: 18px 16px;
    display: flex;
    flex-direction: column;
    gap: 18px;
  }
  .tile > span {
    font: 10px var(--font-mono);
    text-transform: uppercase;
    line-height: 1.5;
  }
  .tile strong {
    font: 38px/1 var(--font-disp);
  }
  .tile small {
    font: 20px var(--font-disp);
  }
  .accent {
    background: var(--acd);
  }
  .token-strip {
    display: flex;
    gap: 28px;
    align-items: center;
    padding: 20px 0;
    border-bottom: 2px solid var(--ink);
    margin-bottom: 28px;
  }
  .token-strip > div {
    display: grid;
    gap: 5px;
    min-width: 150px;
  }
  .token-strip span {
    font: 10px var(--font-mono);
  }
  .token-strip b {
    font: 27px var(--font-disp);
  }
  .token-strip p {
    font-size: 12px;
    color: var(--dim);
    max-width: 410px;
    margin-left: auto;
  }
  .hero {
    display: flex;
    text-align: left;
    align-items: center;
    width: 100%;
    padding: 0;
    gap: 30px;
    margin: 30px 0;
    background: var(--acd);
  }
  .hero:hover {
    background: var(--acd);
    color: var(--ink);
  }
  .hero-portrait {
    height: 245px;
    width: 190px;
    flex: none;
    border-right: 2px solid var(--ink);
  }
  .hero h2 {
    font-size: 50px;
    margin: 5px 0;
  }
  .hero span {
    font: 12px var(--font-mono);
  }
  .hero-number {
    font: 135px var(--font-disp);
    opacity: 0.2;
    margin-left: auto;
    margin-right: 30px;
  }
  section {
    padding: 26px 0;
    border-bottom: 2px solid var(--ink);
  }
  .section-title {
    margin-bottom: 20px;
  }
  .section-title > span {
    color: var(--dim);
  }
  .cast-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
    gap: 16px;
  }
  .cast-card {
    text-align: left;
    padding: 0 0 12px;
  }
  .cast-card h3,
  .cast-card p,
  .cast-card small {
    margin-left: 12px;
    margin-right: 12px;
  }
  .cast-card p {
    font: 11px var(--font-mono);
  }
  .cast-card small {
    font: 10px var(--font-mono);
    line-height: 1.7;
    display: block;
  }
  .cast-image {
    height: 180px;
    position: relative;
    border-bottom: 2px solid var(--ink);
  }
  .cast-image > span {
    position: absolute;
    bottom: 0;
    left: 0;
    background: var(--ink);
    color: var(--bone);
    padding: 4px 10px;
    font: 22px var(--font-disp);
  }
  .scroll {
    overflow: auto;
    min-width: 0;
  }
  /* long breakdown / generation tables scroll inside the page instead of
     stretching it and leaving the neighboring grid cell empty */
  .scroll.cap {
    max-height: 340px;
  }
  .scroll.cap thead th {
    position: sticky;
    top: 0;
    background: var(--bone);
    z-index: 1;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    text-align: left;
    font: 11px var(--font-mono);
  }
  th {
    font-size: 9px;
    text-transform: uppercase;
    color: var(--dim);
    white-space: nowrap;
  }
  th,
  td {
    padding: 12px 10px;
    border-bottom: 1px solid var(--hair);
  }
  td small {
    display: block;
    margin-top: 4px;
    color: var(--dim);
  }
  .model-layout {
    display: grid;
    grid-template-columns: 340px minmax(0, 1fr);
    gap: 30px;
    align-items: start;
  }
  .model-donut {
    max-width: 340px;
    width: 100%;
  }
  .swatch {
    display: inline-block;
    width: 10px;
    height: 10px;
    margin-right: 7px;
  }
  .personas {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 16px;
  }
  .persona {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  .avatar {
    width: 55px;
    height: 65px;
    flex: none;
    border: 2px solid var(--ink);
  }
  .persona p {
    font: 11px var(--font-mono);
    color: var(--dim);
  }
  .facts {
    display: flex;
    gap: 25px;
    flex-wrap: wrap;
    padding: 15px 0;
  }
  .facts > div {
    display: flex;
    flex-direction: column;
    gap: 6px;
    flex: 1;
    min-width: 120px;
  }
  .facts b {
    font: 25px var(--font-disp);
  }
  .facts span {
    font: 10px var(--font-mono);
    color: var(--dim);
  }
  .habit-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(170px, 1fr));
    gap: 28px;
  }
  .two-charts {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 30px;
  }
  .note {
    font: 11px/1.7 var(--font-mono);
    color: var(--dim);
  }
  .detail-heading {
    display: flex;
    align-items: center;
    gap: 20px;
    margin: 24px 0;
  }
  .detail-heading > button:last-child {
    margin-left: auto;
  }
  .detail-portrait {
    width: 95px;
    height: 120px;
    border: 2px solid var(--ink);
  }
  .text-button {
    border: 0;
    padding: 0;
    text-align: left;
    font: inherit;
    text-decoration: underline;
  }
  .error {
    padding: 16px;
    border: 2px solid var(--org);
    font-size: 13px;
  }
  .empty {
    padding: 65px 30px;
    text-align: center;
    border: 2px solid var(--ink);
  }
  .empty a,
  .empty button {
    display: inline-block;
    margin: 15px;
  }
  .usage-controls {
    display: flex;
    align-items: center;
    gap: 18px;
    flex-wrap: wrap;
    margin-bottom: 20px;
  }
  .usage-controls p {
    margin-right: auto;
  }
  .usage-controls label {
    font-size: 12px;
  }
  .range {
    display: flex;
  }
  .range button {
    padding: 8px 12px;
  }
  /* column flow packs the uneven tables tightly — a row grid leaves the
     short cell hanging under its tall neighbor */
  .breakdowns {
    columns: 2;
    column-gap: 30px;
  }
  .breakdowns > section {
    break-inside: avoid;
    padding-top: 10px;
  }
  .chat-list {
    max-height: 480px;
    overflow: auto;
    border-bottom: 1px solid var(--hair);
  }
  .usage-chat {
    border-bottom: 1px solid var(--hair);
  }
  .usage-chat > button {
    width: 100%;
    border: 0;
    display: flex;
    justify-content: space-between;
    padding: 17px 0;
    text-align: left;
    gap: 20px;
  }
  .usage-chat > button span:last-child {
    font: 11px var(--font-mono);
    flex: none;
  }
  .usage-tiles {
    grid-template-columns: repeat(4, minmax(0, 1fr));
  }
  @media (max-width: 1100px) {
    .tiles {
      grid-template-columns: repeat(3, minmax(0, 1fr));
    }
    .usage-tiles {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
    .wc-page {
      padding: 25px 18px;
    }
    .wc-header {
      align-items: start;
    }
    h1 {
      font-size: 46px;
    }
    .model-layout {
      grid-template-columns: 1fr;
    }
    .model-donut {
      margin: auto;
    }
    .breakdowns {
      columns: 1;
    }
    .hero-number {
      font-size: 90px;
    }
    .tools {
      max-width: 260px;
    }
    .token-strip {
      flex-wrap: wrap;
    }
    .token-strip p {
      margin: 0;
      max-width: none;
    }
    .usage-chat > button {
      flex-wrap: wrap;
    }
  }
</style>
