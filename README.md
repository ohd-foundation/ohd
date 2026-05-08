# Open Health Data (OHD)

A decentralized, user-owned protocol for personal health data.

The protocol spec, architecture, and design docs live in [`spec/`](spec/README.md). The implementation was scrapped in favor of a fresh start once the spec is finalized.

## What's here

| Path | What it is |
|---|---|
| [`spec/`](spec/) | The protocol spec — architecture, components, data model, storage format, deployment modes, design files. The canonical source of truth. |
| [`ux-design.md`](ux-design.md) | UX brief for the user-facing apps. Aesthetic and screen-level guidance is current; the architecture section needs a review-pass update. |
| [`claude-architecture-dump.md`](claude-architecture-dump.md) | Mermaid-diagram externalization of the architecture, aligned with the current spec. Used as a review aid; scrap after the spec review pass if not needed. |

## Status

**Spec stabilization.** Implementation cleared out. Once the spec review settles, the implementation plan gets re-created against the five-component model: OHD Storage (core), OHD Connect (personal app), OHD Care (clinical app), OHD Emergency (emergency-services app), OHD Relay (bridge + emergency authority). All five speak the unified OHDC protocol with three auth profiles (self-session, grant token, device token).

## Where to start

- [`spec/README.md`](spec/README.md) — protocol overview and doc index.
- [`spec/docs/01-architecture.md`](spec/docs/01-architecture.md) — how the four components fit together.
- [`spec/docs/deployment-modes.md`](spec/docs/deployment-modes.md) — where the data lives (on-device, self-hosted, custom provider, OHD Cloud).
- [`spec/docs/components/`](spec/docs/components/) — per-component specs (storage, connect, care, emergency, relay).
- [`spec/docs/design/storage-format.md`](spec/docs/design/storage-format.md) — the on-disk format.
- [`spec/docs/glossary.md`](spec/docs/glossary.md) — every term defined once.
