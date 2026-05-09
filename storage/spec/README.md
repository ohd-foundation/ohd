# `storage/spec/` — Storage-owned design docs

This directory holds the design documents that **OHD Storage owns or
implements**, copied verbatim from the global spec at
`../../spec/docs/design/`. They are the contract this component implements.

The originals in `../../spec/` are the canonical source. The copies here
travel with the storage codebase so a developer working only on this
component has the relevant specs at hand. When the global spec changes, these
copies need to be re-synced (a CI check could enforce identity later).

## File index

| File | Spec area | Why storage owns it |
|---|---|---|
| [`storage-format.md`](storage-format.md) | On-disk format — schema, sample blocks, channel registry, grants, sync, deployment modes | Storage *is* the file format. This is the contract. |
| [`ohdc-protocol.md`](ohdc-protocol.md) | OHDC v0 wire spec — services, messages, error model, pagination, idempotency, filter language, streaming, `.proto` source | Storage exposes OHDC; this is its only external surface. |
| [`sync-protocol.md`](sync-protocol.md) | Cache ↔ primary sync wire spec — `SyncService` RPCs, frame stream, watermarks, attachment payload sync, grant lifecycle out-of-band | Sync is intra-storage replication; the wire format and the convergence rules both live here. |
| [`encryption.md`](encryption.md) | Per-user file encryption — key hierarchy, recovery (BIP39), multi-device pairing, key rotation, export encryption | Storage holds the keys, derives them at unlock, and zeros them on lock. |
| [`privacy-access.md`](privacy-access.md) | Identity, permissions, audit trails | Storage runs the resolution algorithm and writes the audit. |
| [`conformance.md`](conformance.md) | OHDC v0 conformance corpus — what claiming "v0" means | Storage is the reference implementation; conformance is its yardstick. |
| [`data-model.md`](data-model.md) | Conceptual event vocabulary | The standard registry storage embeds is derived from this. |

## Cross-cutting concerns referenced but not owned by storage

These live in `../../spec/docs/design/` only; storage uses them but doesn't
implement them:

- `auth.md` — OAuth flows, token formats, account-join modes. Storage
  validates tokens and resolves them to one of three auth profiles, but the
  identity layer's flows are spec'd in the global doc.
- `care-auth.md` — operator-side concerns for OHD Care.
- `emergency-trust.md` — authority cert chain (Fulcio + X.509 + Rekor).
- `relay-protocol.md` — relay tunnel wire spec; storage is one client of
  this protocol, not its owner.
- `notifications.md` — push delivery (FCM/APNs/email). Storage emits
  notification events into its system DB; the delivery pipeline is shared.
- `deployment.md` — Docker Compose, Hetzner, Caddy. Operator concerns,
  layered above storage.

## Relationship to `SPEC.md`

This component's [`../SPEC.md`](../SPEC.md) is the *implementation-ready*
distillation of the docs in this directory — what storage actually has to
build, in the shape the implementation phase will pick up. The docs here are
the source of authority; `SPEC.md` is the working summary.
