<script lang="ts">
  import { onMount } from "svelte";
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import { wackchatterApi as api } from "$lib/api";
  import type { Library, Stats, Usage, Call } from "$lib/wackchatter/types";
  import LibraryView from "$lib/wackchatter/LibraryView.svelte";
  import UsageView from "$lib/wackchatter/UsageView.svelte";
  import { Auras, LIME } from "$lib/wackchatter/auras.svelte";
  import { forgetPortraits } from "$lib/wackchatter/portraits";
  import { readPref, writePref } from "$lib/prefs";
  import "$lib/wackchatter/wackchatter.css";

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
  const currentLibrary = $derived(libraries.find((l) => l.id === library));
  const auras = new Auras();
  $effect(() => {
    if (library && stats) auras.request(library, stats.cast.map((c) => c.character_id));
  });
  const accent = $derived(
    character && stats ? auras.of(library, character, stats.colors.characters) : LIME,
  );
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
<div
  class="wc wc-page up"
  class:tinted={Boolean(character)}
  style="--aura:{accent.fill};--aura-ink:{accent.ink};--aura-wash:{accent.wash}"
>
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
        >{:else if currentLibrary}<span class="library-tag">{currentLibrary.name}</span>{/if}
      <button
        class="wc-btn"
        onclick={() => {
          forgetPortraits();
          auras.reset();
          void refreshLibraries();
          void load(library, character, group, days, allUsage);
        }}
        disabled={loading}>↻ Refresh</button
      >
      <button class="wc-btn" onclick={() => open()} disabled={!library}>Open WackChatter ↗</button>
    </div>
  </header>
  <div class="tabs">
    <button class:active={tab === "library"} onclick={() => navigate({ tab: null })}
      >Library <small>ALL TIME</small></button
    ><button
      class:active={tab === "usage"}
      onclick={() => navigate({ tab: "usage", character: null, group: null })}>Usage</button
    ><span>Local data · Read-only</span>
  </div>
  {#if actionError}<p class="error" role="alert">{actionError}</p>{/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if loading}<p class="empty" role="status">Reading your library…</p>
  {:else if tab === "library"}
    {#if !stats && error}<div class="empty">
        <h2>Library unavailable</h2>
        <p>
          Reconnect the library or start WackChatter with it, then refresh. Recorded usage
          remains available in the Usage tab.
        </p>
      </div>
    {:else if !stats}<div class="empty">
        <h2>Your stories start here</h2>
        <p>
          Start WackChatter once to discover its library. Stats will then work even when
          WackChatter is closed.
        </p>
        <a class="wc-btn" href="https://github.com/Intelios/wackchatter" target="_blank" rel="noreferrer"
          >Get WackChatter ↗</a
        ><button class="wc-btn" onclick={() => navigate({ tab: "usage", character: null, group: null })}
          >View recorded usage</button
        >
      </div>
    {:else}
      {#key `${library}|${character}|${group}`}
        <LibraryView {library} {stats} {character} {group} {auras} {accent} {navigate} {open} />
      {/key}
    {/if}
  {:else if usage}
    <UsageView
      {usage}
      {stats}
      {library}
      {auras}
      bind:allUsage
      bind:days
      {expanded}
      {calls}
      {callsLoading}
      {expand}
      {conversationName}
    />
  {/if}
</div>

<style>
  .wc-page {
    /* edge-to-edge like the rest of the app — no centered column */
    padding: 36px clamp(22px, 1.8vw, 40px) 70px;
    min-height: calc(100vh - 52px);
    transition: background-color 0.4s;
  }
  /* a character's profile takes on their aura, faintly, behind everything */
  .wc-page.tinted {
    background: linear-gradient(180deg, var(--aura-wash) 0, var(--bone) 720px);
  }
  .wc-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 22px;
    margin-bottom: 30px;
  }
  .eyebrow {
    font: 11px var(--font-mono);
    letter-spacing: 1.5px;
    text-transform: uppercase;
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
    background: var(--aura);
    border: 2px solid var(--ink);
    transition: background-color 0.4s;
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
  .library-tag {
    font: 12px var(--font-mono);
    padding: 10px;
  }
  .tabs {
    display: flex;
    border-bottom: 2px solid var(--ink);
    margin-bottom: 28px;
  }
  .tabs button {
    background: transparent;
    border: 2px solid var(--ink);
    border-bottom: 0;
    padding: 14px 25px;
    font: 600 12px var(--font-ui);
    text-transform: uppercase;
    letter-spacing: 1px;
    color: var(--ink);
    cursor: pointer;
    border-radius: 0;
    margin-right: -2px;
  }
  .tabs button:hover {
    background: rgba(13, 13, 11, 0.08);
  }
  .tabs button.active {
    background: var(--ink);
    color: var(--bone);
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
  .empty h2 {
    font: 30px var(--font-disp);
    margin: 0 0 12px;
  }
  .empty a,
  .empty button {
    display: inline-block;
    margin: 15px;
  }
  @media (max-width: 1100px) {
    .wc-page {
      padding: 25px 18px;
    }
    .wc-header {
      align-items: start;
    }
    h1 {
      font-size: 46px;
    }
    .tools {
      max-width: 260px;
    }
  }
</style>
