import { describe, expect, test } from "bun:test";
import { tokenTrailDestination } from "../src/lib/wackchatter/navigation";
describe("TokenTrail handoffs", () => {
  test("preserves library and encoded character identity", () => {
    expect(
      tokenTrailDestination(
        "tokentrail://wackchatter?library=2026%3A01&character=A%20%26%20B.png",
      ),
    ).toBe("/wackchatter?library=2026%3A01&character=A+%26+B.png");
  });
  test("only routes known destinations", () => {
    for (const url of [
      "file:///etc/passwd",
      "https://wackchatter",
      "tokentrail://settings",
      "tokentrail://wackchatter/../../settings",
      "tokentrail://evil@wackchatter",
    ])
      expect(tokenTrailDestination(url)).toBeNull();
  });
  test("never forwards arbitrary paths or actions", () => {
    expect(
      tokenTrailDestination(
        "tokentrail://wackchatter?path=/etc/passwd&action=delete",
      ),
    ).toBe("/wackchatter");
  });
});
