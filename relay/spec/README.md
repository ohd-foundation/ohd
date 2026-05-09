# OHD Relay — Spec Index

Snapshot of canonical design docs that pin the relay's wire and trust contracts. The authoritative copies live in `../../spec/docs/design/`; these are vendored here so the implementation crate has everything it needs in one tree.

| File | Source | What it pins |
|---|---|---|
| [`relay-protocol.md`](./relay-protocol.md) | `spec/docs/design/relay-protocol.md` | Tunnel framing, TLS-through-tunnel cert pin, registration / heartbeat / deregister, session multiplexing, LAN fast-path |
| [`emergency-trust.md`](./emergency-trust.md) | `spec/docs/design/emergency-trust.md` | Authority cert chain (Fulcio + X.509 + Rekor), 24h org cert refresh, signed `EmergencyAccessRequest`, patient-phone verification |
| [`notifications.md`](./notifications.md) | `spec/docs/design/notifications.md` | Push delivery (FCM / APNs / email), no-PHI payload contract, tunnel-wake silent push for phone storage |

For the component-level overview (purpose, operator landscape, persistence model, what the relay does and doesn't do), see [`../../spec/docs/components/relay.md`](../../spec/docs/components/relay.md).

For local distillations targeted at this crate's implementer, see [`../SPEC.md`](../SPEC.md).
