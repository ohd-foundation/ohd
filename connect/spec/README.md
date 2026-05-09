# Connect spec snapshot

Local copies of the canonical spec files relevant to OHD Connect. The source of
truth lives in [`../../spec/`](../../spec/); these are pinned snapshots so the
Connect tree is self-contained for the implementation phase.

| File | Source | Purpose for Connect |
|---|---|---|
| [`auth.md`](auth.md) | `../../spec/docs/design/auth.md` | OIDC self-session, OAuth code-flow + PKCE, device flow, dynamic client registration, opaque token formats, system-DB tables, on-device sub-modes. |
| [`notifications.md`](notifications.md) | `../../spec/docs/design/notifications.md` | Push delivery — FCM (Android), APNs (iOS), Web Push, email fallback. No-PHI payload contract. Categories Connect must handle: `pending_write`, `emergency`, `tunnel_wake`, `grant_expiring`, `access`. |
| [`mcp-servers.md`](mcp-servers.md) | `../../spec/docs/research/mcp-servers.md` | MCP server design + tool definitions. **Connect MCP only** is in scope here — the Care MCP and Emergency MCP sections are out of scope for this component. |
| [`health-connect.md`](health-connect.md) | `../../spec/docs/research/health-connect.md` | Android Health Connect bridge — record types, permissions, change tokens, sync architecture. Android-only. |
| [`openfoodfacts.md`](openfoodfacts.md) | `../../spec/docs/research/openfoodfacts.md` | OpenFoodFacts API for barcode-driven food logging — endpoint, fields, caching, OHD event mapping. |
| [`barcode-scanning.md`](barcode-scanning.md) | `../../spec/docs/research/barcode-scanning.md` | Camera-based barcode scanner — ML Kit + CameraX (Android); Vision (iOS); `BarcodeDetector` (web). |
| [`screens-emergency.md`](screens-emergency.md) | `../../spec/design/screens-emergency.md` | Emergency / break-glass UX. Connect hosts the **patient-side** screens (settings, dialog, cases tab). The OHD Emergency component owns the responder-side screens. |

## What Connect implements vs. references

Connect is the **personal-side** consumer of OHDC. It holds self-session
tokens, never grant tokens (those are issued by Connect to other parties) or
device tokens (those are held by sensor backends, the Health Connect bridge
service, etc.).

Connect does **not** define:

- The OHDC wire protocol — that's `../../spec/docs/design/ohdc-protocol.md`.
- The on-disk storage format — that's `../../spec/docs/design/storage-format.md`.
- The relay protocol — that's `../../spec/docs/design/relay-protocol.md`.
- Emergency trust / cert-chain semantics — that's `../../spec/docs/design/emergency-trust.md`.
- The Care or Emergency MCP surfaces — see those components.

Connect is a **client** of all of the above. The relevant pieces of each that
Connect must speak to are summarized in [`../SPEC.md`](../SPEC.md).

## When the source spec changes

If a file in `../../spec/` is updated and Connect needs to track it, refresh
the corresponding file here with `cp` and note the date in
[`../STATUS.md`](../STATUS.md). Don't edit these files in place — keep them as
verbatim snapshots.
