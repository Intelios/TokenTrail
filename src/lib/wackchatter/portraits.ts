import { wackchatterApi } from "$lib/api";

const cache = new Map<string, Promise<string>>();

/** One IPC read per portrait: cards, chips and aura extraction share it. */
export function portraitSrc(library: string, id: string, persona = false): Promise<string> {
  const key = `${persona ? "p" : "c"}:${library}:${id}`;
  let hit = cache.get(key);
  if (!hit) {
    hit = wackchatterApi.portrait(library, id, persona).catch(() => "");
    cache.set(key, hit);
  }
  return hit;
}

/** A manual refresh re-reads portraits, so a replaced card shows its new art. */
export function forgetPortraits() {
  cache.clear();
}
