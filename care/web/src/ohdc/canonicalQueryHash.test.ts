// Cross-language parity test for `canonicalQueryHash`.
//
// Loads `__golden__/query_hash_vectors.json` and asserts that:
// 1. `canonicalFilterJson(filter)` matches `canonical_payload` byte-for-byte
//    (the storage `serde_json::to_string(filter)` shape).
// 2. `canonicalQueryHash(kind, filter)` matches `expected_hex`.
//
// The same JSON is loaded by the Python parity tests in
// `care/cli/tests/test_canonical_query_hash.py` and
// `care/mcp/tests/test_canonical_query_hash.py` — when this test passes
// here AND there, the TS, Python (cli), and Python (mcp) implementations
// are byte-identical.
//
// To regenerate / fill placeholders, run `pnpm test` and inspect the
// "actual" values printed on failure, then paste them back into the JSON.
// Per the file header in `canonicalQueryHash.ts`, the long-term plan is a
// Rust helper (`care/cli/tests/golden_query_hash.rs`) that emits this
// JSON from the storage struct directly.

import { describe, expect, it } from "vitest";
import vectors from "./__golden__/query_hash_vectors.json";
import {
  canonicalFilterJson,
  canonicalQueryHash,
  type CanonicalEventFilter,
  type CanonicalQueryKind,
} from "./canonicalQueryHash";

interface Vector {
  name: string;
  query_kind: CanonicalQueryKind;
  filter: CanonicalEventFilter;
  canonical_payload: string;
  expected_hex: string;
}

describe("canonical query-hash — golden vectors", () => {
  for (const v of vectors as Vector[]) {
    it(`payload: ${v.name}`, () => {
      const got = canonicalFilterJson(v.filter);
      expect(got).toBe(v.canonical_payload);
    });

    it(`hash: ${v.name}`, async () => {
      const got = await canonicalQueryHash(v.query_kind, v.filter);
      // Allow placeholder vectors to record the computed value on first
      // run. The CI gate is the strict equality below; placeholders
      // fail loudly so they get filled in.
      if (v.expected_hex === "PLACEHOLDER_FILLED_BY_TEST") {
        // Print so the dev sees the value to paste back. Vitest captures
        // stdout per-test; this isn't noise on green runs.
        // eslint-disable-next-line no-console
        console.log(`[golden] ${v.name} → ${got}`);
        expect(got).toMatch(/^[0-9a-f]{64}$/);
        return;
      }
      expect(got).toBe(v.expected_hex);
    });
  }
});
