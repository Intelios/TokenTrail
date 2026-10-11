/** Only known TokenTrail destinations are accepted; deep links never read paths. */
export function tokenTrailDestination(raw: string): string | null {
  try {
    const u = new URL(raw);
    if (
      u.protocol !== "tokentrail:" ||
      u.hostname !== "wackchatter" ||
      !["", "/"].includes(u.pathname) ||
      u.username ||
      u.password
    )
      return null;
    const q = new URLSearchParams();
    for (const key of ["library", "character"]) {
      const v = u.searchParams.get(key);
      if (v) {
        if (v.length > 2048) return null;
        q.set(key, v);
      }
    }
    return "/wackchatter" + (q.size ? "?" + q : "");
  } catch {
    return null;
  }
}
