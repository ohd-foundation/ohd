# OHD Care

> Reference clinical-side application of the [OHD](../README.md) protocol. A real, lightweight, EHR-shaped consumer that healthcare professionals (doctors, specialists, nurses, paramedics, therapists) use to access and contribute to a patient's OHD via OHDC grant tokens.

OHD Care is one of the five OHD components, alongside [Storage](../storage/), [Connect](../connect/), [Emergency](../emergency/), and [Relay](../relay/). The full system architecture is documented in [`../spec/docs/01-architecture.md`](../spec/docs/01-architecture.md).

## What Care is

A multi-patient clinical app speaking OHDC under grant-token auth. Distinctive properties:

- **Multi-patient roster** — every patient who has granted the operator access. Active-patient context drives every operation.
- **Visit prep** — opens a patient and pre-fetches data slices likely to matter for the chief complaint (temp series, recent meds, related symptoms, prior-visit summary).
- **Write-with-approval** — clinical submissions (notes, labs, prescriptions, referrals) flow into the patient's pending queue per the grant's `approval_mode`.
- **Case-aware** — encounters span cases (admission, outpatient visit, inherited EMS handoff). Cases are first-class in the UI and MCP, with predecessor/parent inheritance, handoff, auto-close + reopen tokens.
- **Two-sided audit** — every OHDC call lands in both the patient's audit log (visible in OHD Connect) and Care's operator-side audit (visible to compliance).
- **Trust boundary** — Care holds **grants**, not patient data. Revocation from Connect is immediate; the operator's local cache becomes their HIPAA / GDPR responsibility from that moment.

OHD Care is **not** Epic / Cerner. No billing, no scheduling, no claims, no DICOM viewer, no HL7 / FHIR mapping suite. It's the lightweight reference for OHD-native clinical workflows, deployable in an afternoon, that operators run alongside (or instead of) larger EHRs that don't speak OHDC.

The full conceptual spec is [`../spec/docs/components/care.md`](../spec/docs/components/care.md). The implementation-ready spec for this repo is [`./SPEC.md`](./SPEC.md).

## Forms in this repo

| Subdir | Form | Stack |
|---|---|---|
| [`web/`](./web) | Operator-facing SPA: roster, per-patient view, chart builder, audit transparency, write-with-approval UI | Vite + React + TypeScript, `@ohd/shared-web` workspace package |
| [`mcp/`](./mcp) | Care MCP server — multi-patient LLM workflow with active-patient context | Python + FastMCP, `ohd-shared` |
| [`cli/`](./cli) | `ohd-care` CLI — patients / use / temperature / submit | Python (uv-style `pyproject.toml`), `ohd-shared` |
| [`demo/`](./demo) | End-to-end write-with-approval demo (storage + Connect CLI + Care web SPA) | shell + driver script |
| [`deploy/`](./deploy) | Reference Docker Compose + Caddy for an operator's domain | Docker, Caddy |
| [`spec/`](./spec) | Local snapshots of the canonical spec docs Care implements against | Markdown |

The Python forms (CLI + MCP) share helpers via [`ohd-shared`](../packages/python/ohd-shared) (transport, canonical query hash, OAuth proxy, generated proto stubs). The web SPA shares OIDC + store hooks with `connect/web` and `emergency/dispatch` via [`@ohd/shared-web`](../packages/web/ohd-shared-web).

## Deployment shapes

OHD Care is deployable in many shapes — same image, different config. The reference Compose file is in [`./deploy`](./deploy).

| Shape | Operator | Notes |
|---|---|---|
| **Single-practitioner** | One clinician, one VPS | Docker Compose on a small VPS (e.g. Hetzner cx22). Practitioner is the operator and the IT team. Local key file for grant-token KMS; PBKDF2-derived from a passphrase entered at service start. |
| **Small clinic** | A few clinicians, one VPS | Same Compose stack, multiple operator accounts via clinic OIDC (Google Workspace / Microsoft 365). Optional Postgres for the operator-side state when staff > ~10. |
| **Hospital department** | Department IT | Deployed inside hospital infra; integrates with hospital ADFS/Entra; coexists with main EHR (Epic / Cerner) which doesn't speak OHDC. KMS via hospital HSM. |
| **Mobile / ambulance** | EMS station | Operator app on a tablet, with break-glass grants pre-authorized on patient OHDs. Tighter session timeouts. |
| **Clinical trial site** | CRO / sponsor | Per-study deployment; participants grant the trial's CRO with study-scoped (case-bound) grants. |
| **Direct-pay / boutique practice** | Individual / small group | Primary clinical tool; no main EHR. Operator's OIDC is whatever they already use. |

The deploy story is intentionally lightweight — comparable to spinning up Bitwarden or Nextcloud.

## Where this fits in the global spec

- **Component spec** (what Care is and does): [`../spec/docs/components/care.md`](../spec/docs/components/care.md)
- **Operator auth & grant vault**: [`../spec/docs/design/care-auth.md`](../spec/docs/design/care-auth.md) — also in [`./spec/care-auth.md`](./spec/care-auth.md)
- **OHDC protocol** Care speaks: [`../spec/docs/components/connect.md`](../spec/docs/components/connect.md), wire spec at [`../spec/docs/design/ohdc-protocol.md`](../spec/docs/design/ohdc-protocol.md)
- **Relay path** for patients on phones / behind NAT: [`../spec/docs/components/relay.md`](../spec/docs/components/relay.md), wire spec at [`../spec/docs/design/relay-protocol.md`](../spec/docs/design/relay-protocol.md)
- **Emergency handoff** into Care (predecessor case from EMS): [`../spec/docs/components/emergency.md`](../spec/docs/components/emergency.md), trust model at [`../spec/docs/design/emergency-trust.md`](../spec/docs/design/emergency-trust.md)
- **MCP design** (Care-relevant sections): [`../spec/docs/research/mcp-servers.md`](../spec/docs/research/mcp-servers.md) — also in [`./spec/mcp-servers.md`](./spec/mcp-servers.md)
- **Glossary** (every OHD term): [`../spec/docs/glossary.md`](../spec/docs/glossary.md)
- **UX design vocabulary**: [`../ux-design.md`](../ux-design.md)

## Getting started

Each subdir has its focused `README.md`. For an end-to-end clinical workflow (storage server seeded with patient data, Connect CLI logging, Care web SPA submitting a clinical note that the patient then approves), run [`./demo/`](./demo/):

```sh
bash care/demo/run.sh
# then in another terminal:
cd care/web && pnpm dev
# open the printed http://localhost:5173/?token=ohdg_… URL
```

See [`./STATUS.md`](./STATUS.md) for the per-form-factor state.

## Test

| Form | Command |
|---|---|
| `cli/` | `cd cli && uv run pytest` |
| `mcp/` | `cd mcp && uv run pytest` |
| `web/` | `cd web && pnpm test` |

## License

Dual-licensed under your choice of [Apache-2.0](../spec/LICENSE-APACHE) or [MIT](../spec/LICENSE-MIT), matching the wider OHD project.
