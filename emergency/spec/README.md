# OHD Emergency — Spec Index (component-local)

> Local copies of the canonical spec material relevant to OHD Emergency. The single source of truth is `../../spec/`; these files are snapshots for implementer convenience. If they drift, the global spec wins.

## Contents

| File | Source in global spec | What it covers |
|---|---|---|
| [`emergency-trust.md`](emergency-trust.md) | `spec/docs/design/emergency-trust.md` | Authority cert chain (Fulcio + X.509 + Rekor), short-lived org/responder certs, signed `EmergencyAccessRequest`, patient-phone verification algorithm. |
| [`screens-emergency.md`](screens-emergency.md) | `spec/design/screens-emergency.md` | Designer-handoff: patient-side (Connect) settings + dialog, responder-side (Emergency) tablet + dispatch console screens. |
| [`mcp-servers.md`](mcp-servers.md) | `spec/docs/research/mcp-servers.md` | MCP server design research. **Emergency MCP is a brief sketch in [`../../spec/docs/components/emergency.md`](../../spec/docs/components/emergency.md) "OHD Emergency MCP" section** — the bulk of this doc covers Connect MCP and Care MCP and is included here only for shared context (FastMCP framing, transport choices, OAuth proxy patterns). |

## Where the actual contracts live

| Topic | Where |
|---|---|
| Component spec (this dir's purpose) | [`../../spec/docs/components/emergency.md`](../../spec/docs/components/emergency.md) |
| Architecture context | [`../../spec/docs/01-architecture.md`](../../spec/docs/01-architecture.md) |
| Glossary | [`../../spec/docs/glossary.md`](../../spec/docs/glossary.md) |
| Relay component (emergency-authority mode) | [`../../spec/docs/components/relay.md`](../../spec/docs/components/relay.md) |
| OHDC protocol | [`../../spec/docs/design/ohdc-protocol.md`](../../spec/docs/design/ohdc-protocol.md) |

## Important: the relay is not in this repo

OHD Emergency is the **consumer** of an emergency-authority relay. The relay binary itself — including emergency-authority mode, the Fulcio integration, the cert refresh daemon, and the `DeliverEmergencyRequest` signing path — lives in `../../relay/`, not here.

Emergency depends on a relay deployment configured with `authority-mode: true`. See [`../deploy/docker-compose.yml`](../deploy/docker-compose.yml) for the reference compose topology and [`../SPEC.md`](../SPEC.md) for the trust boundary diagram.

## Component scope reminder

OHD Emergency builds:

- The paramedic tablet app (Android-first, iOS later) — [`../tablet/`](../tablet/)
- The dispatch console web app — [`../dispatch/`](../dispatch/)
- The Emergency MCP server — [`../mcp/`](../mcp/)
- The operator-side CLI — [`../cli/`](../cli/)
- A reference operator-side records DB schema — [`../SPEC.md`](../SPEC.md) "Operator-side records"
- A reference deployment — [`../deploy/`](../deploy/)

It does NOT build the relay, Fulcio, Rekor, or the patient-phone break-glass dialog (that's OHD Connect).
