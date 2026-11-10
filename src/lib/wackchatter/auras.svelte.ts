import { portraitPalette, resolveAura, type Aura } from "./aura";
import { portraitSrc } from "./portraits";

/** Lime-on-ink: the library's own accent when no character is in focus. */
export const LIME: Aura = { fill: "#c8e600", ink: "#0d0d0b", wash: "#e8e4d9" };

function seed(id: string) {
  let h = 0;
  for (const ch of id) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  return h;
}

/**
 * Portrait palettes arrive one by one; the map is reactive so every chart that
 * coloured a character with its fallback repaints once the real colour lands.
 * The fallback is seeded by identity, not rank, so a character keeps one colour
 * on the overview and on their own profile.
 */
export class Auras {
  #palettes = $state<Record<string, string[]>>({});
  #asked = new Set<string>();

  request(library: string, ids: readonly string[]) {
    for (const id of ids) {
      const key = `${library}:${id}`;
      if (!id || this.#asked.has(key)) continue;
      this.#asked.add(key);
      void portraitSrc(library, id)
        .then((src) => (src ? portraitPalette(key, src) : []))
        .then((palette) => {
          this.#palettes = { ...this.#palettes, [key]: palette };
        });
    }
  }

  of(library: string, id: string, pinned: Record<string, string[]> | undefined): Aura {
    return resolveAura(pinned?.[id], this.#palettes[`${library}:${id}`], seed(id));
  }

  reset() {
    this.#asked.clear();
    this.#palettes = {};
  }
}
