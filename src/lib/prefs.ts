// localStorage-backed view preferences (e.g. the remembered range on Models).
// Reads fall back to the initial value when storage is unavailable or holds a
// value that is no longer one of the valid options.
export function readPref<T>(key: string, initial: T, valid: (v: unknown) => boolean): T {
  try {
    const raw = localStorage.getItem(key);
    if (raw === null) return initial;
    const parsed: unknown = JSON.parse(raw);
    return valid(parsed) ? (parsed as T) : initial;
  } catch {
    return initial;
  }
}

export function writePref(key: string, value: unknown): void {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // storage unavailable (private mode, quota) — the preference just won't persist
  }
}
