# OHD Care — local spec snapshots

> Frozen copies of the canonical OHD spec docs that drive Care's implementation. The authoritative versions live in `../../spec/`. These copies sit in-tree so the Care implementation phase has a stable reference point during the v0.1 build.

## Files

- [`care-auth.md`](./care-auth.md) — copy of `spec/docs/design/care-auth.md`. The full design for operator authentication into Care, the per-patient grant-token vault (schema, lifecycle, encryption-at-rest), two-sided audit, talking to relay-mediated patient storage, and the OHDC RPCs Care uses. Read this first when implementing operator login, the `Patients → Add patient` flow, the grant cache, or the audit pipeline.

- [`mcp-servers.md`](./mcp-servers.md) — copy of `spec/docs/research/mcp-servers.md`. Project-wide MCP research; **the Care-relevant sections are**:
  - "Two distinct MCP servers" → "**Care MCP — data retrieval and analysis**" (purpose, who runs it, auth, transport).
  - "Care MCP: auto-generated from FastAPI" — note the doc's status caveat: the FastAPI-auto-generation model is superseded by the Rust + Connect-RPC architecture. The hand-written-tools approach (Approach B) is what Care MCP follows in v1.
  - "Care MCP: the hand-written high-level tools" — the v1 starter tool catalog (`query_latest`, `summarize`, `correlate`, `get_medications_taken`, `get_food_log`, `find_patterns`, `chart`).
  - "Authentication with FastMCP's OAuth proxy" — operator-side OAuth wiring.
  - "Tool catalog management (Phase 2+)" — search transforms when the catalog grows.

  Sections that **don't** apply to Care: "Connect MCP — data entry" (that's the Connect component) and the Connect-MCP tool listings (`log_symptom`, `log_meal`, etc.).

## Care-specific tools (extension of the research catalog)

The catalog in `mcp-servers.md` predates the multi-patient model. Care MCP adds:

- **Patient management**: `list_patients`, `switch_patient(label)`, `current_patient`.
- **Write-with-approval**: `submit_lab_result`, `submit_measurement`, `submit_observation`, `submit_clinical_note`, `submit_prescription`, `submit_referral`.
- **Workflow**: `draft_visit_summary`, `compare_to_previous_visit`, `find_relevant_context_for_complaint(complaint)`.
- **Cases**: `open_case`, `close_case`, `handoff_case`, `list_cases`.

The full Care component spec is in [`../../spec/docs/components/care.md`](../../spec/docs/components/care.md). The Care implementation specification (distilled) is in [`../SPEC.md`](../SPEC.md).

## Re-syncing these copies

When the canonical spec changes:

```sh
cp ../../spec/docs/design/care-auth.md ./care-auth.md
cp ../../spec/docs/research/mcp-servers.md ./mcp-servers.md
```

These snapshots are convenience, not source of truth. If they drift from `../../spec/`, the canonical version wins.
