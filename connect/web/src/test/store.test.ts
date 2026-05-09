import { describe, it, expect, beforeEach } from "vitest";
import { _resetForTesting, _setSnapshotForTesting, getRecentEvents, getSnapshot, GRANT_TEMPLATES } from "../ohdc/store";
import { crockfordToBytes, ulidToCrockford } from "../ohdc/client";

beforeEach(() => {
  _resetForTesting();
});

describe("OHDC store — selectors", () => {
  it("getRecentEvents returns at most `limit` from the snapshot", () => {
    _setSnapshotForTesting({
      ready: true,
      events: Array.from({ length: 5 }, (_, i) => ({
        // Minimal Event shape that store.ts accesses; full type from gen.
        timestampMs: BigInt(Date.now() - i * 60_000),
        eventType: "std.blood_glucose",
        channels: [],
      } as unknown as ReturnType<typeof getRecentEvents>[number])),
    });
    const got = getRecentEvents(3);
    expect(got).toHaveLength(3);
  });

  it("getSnapshot reflects updates via _setSnapshotForTesting", () => {
    _setSnapshotForTesting({ ready: true, error: "boom" });
    expect(getSnapshot().error).toBe("boom");
  });
});

describe("Grant templates", () => {
  it("ship five templates with sane defaults", () => {
    const ids = Object.keys(GRANT_TEMPLATES);
    expect(ids.sort()).toEqual([
      "emergency_break_glass",
      "primary_doctor",
      "researcher",
      "specialist_visit",
      "spouse_family",
    ]);
    for (const id of ids) {
      const t = GRANT_TEMPLATES[id as keyof typeof GRANT_TEMPLATES];
      expect(t.label.length).toBeGreaterThan(0);
      expect(["allow", "deny"]).toContain(t.defaultAction);
      expect(["always", "auto_for_event_types", "never_required"]).toContain(t.approvalMode);
    }
  });
});

describe("ULID Crockford codec", () => {
  it("round-trips a known ULID byte sequence", () => {
    // Pick a deterministic 16-byte payload.
    const bytes = new Uint8Array(16);
    for (let i = 0; i < 16; i++) bytes[i] = i * 13;
    const enc = ulidToCrockford(bytes);
    expect(enc).toHaveLength(26);
    const back = crockfordToBytes(enc);
    expect(Array.from(back)).toEqual(Array.from(bytes));
  });

  it("returns empty string for missing or wrong-length input", () => {
    expect(ulidToCrockford(undefined)).toBe("");
    expect(ulidToCrockford(new Uint8Array(15))).toBe("");
  });
});
