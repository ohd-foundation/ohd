# Component: OHD Emergency

> Reference, real, lightweight emergency-services consumer of OHDC. The professional-side counterpart to OHD Care for emergency response. Designed for EMS organizations, hospital emergency departments, mobile-care services, and any operator who needs OHD-data-centric emergency workflows.

## What OHD Emergency is

OHD Emergency is the canonical emergency-services-side application of OHD: a multi-actor, time-critical, mobile-first clinical app that paramedics, dispatchers, ER triage staff, and other emergency responders use to access patient data via break-glass and to record interventions during an emergency case. Like OHD Care, it speaks OHDC; what makes it specific is the emergency workflow shape — break-glass discovery via BLE proximity, certified-authority cert chain, transient case-bound grants, time-pressured UX, ambulance-tablet form factor.

OHD Emergency is **not**:

- Trying to replace ImageTrend, ESO, or NEMSIS-compliant ePCR systems.
- A full computer-aided dispatch (CAD) suite — vehicle dispatch, GPS routing, and resource allocation are CAD's job.
- A hospital ER information system. The receiving ER uses OHD Care (or their own EHR with OHDC support); OHD Emergency hands the case off.
- Personnel scheduling, shift management, certification tracking — operator's HR systems.
- A demo. It's intended to be deployed and used.

OHD Emergency **is**:

- A real, open-source, lightweight clinical app focused on the OHD-data-centric emergency-response workflow.
- The reference implementation that an EMS organization without a major vendor platform can deploy directly. Or that a dispatching software vendor can use as the integration point for OHDC support.
- The pattern showing what a "patient-owned-record" emergency-response system looks like operationally.

## The value story

A paramedic crew arrives at a scene. The patient is conscious but disoriented. The patient has OHD Connect installed; OHD Emergency is on the paramedic's tablet.

**Without OHD**: paramedic asks "do you take any medications? do you have any allergies? when did you last eat? have you ever had a heart problem?" Patient guesses or doesn't remember. Paramedic radios the ER with "patient on unknown medications, denies allergies (uncertain)." ER receives the patient with a partial picture.

**With OHD Emergency**:

1. Paramedic's tablet picks up the patient's BLE beacon (or the patient taps their phone to the tablet).
2. Paramedic taps "Break glass" on their device. The request is signed by their EMS station's relay and sent to the patient's phone.
3. Patient's phone shows the dialog: "EMS Prague Region requesting access — Approve / Reject". Patient approves (conscious case) or 30s timeout fires with default-allow (unconscious case).
4. Paramedic's tablet receives the emergency profile: medications + last taken, allergies, blood type, recent vitals (24h history), active diagnoses. No mental health data. No sexual health data. Per the patient's pre-configured emergency profile.
5. During transport, paramedic logs interventions: vitals every 5 minutes, drugs administered, observations. All written into the patient's OHD as ordinary events; the case includes them automatically because its filter matches the responder's `device_id` and the case's time range.
6. Arriving at the ER, paramedic does a handoff: the case is transferred to the ER's OHD Care; their grant ends, ER's grant starts. ER sees the EMS case as predecessor (they can read what EMS recorded).
7. Patient regains capacity later, opens OHD Connect, sees: "EMS Prague Region had access from 14:23 to 15:08. They recorded 14 events. Handed off to Motol ER. [Review case]."

That's the visit shape OHD Emergency exists to make routine.

## Forms

### OHD Emergency mobile (paramedic tablet)

The primary form. Native Android (Kotlin) and iOS (Swift), tablet-first (ambulance-tablet typical). Authenticated to the operator's relay; talks OHDC via the relay.

Functions:

- **Patient discovery** — BLE scan for nearby OHD beacons. Manual entry of patient identifiers if BLE isn't available.
- **Break-glass initiation** — single-tap from the patient row. Shows the dialog flow status (waiting for patient approval / countdown / approved / rejected / auto-granted via timeout).
- **Patient view** — once granted, displays the emergency profile in a paramedic-optimized layout: critical info (allergies, blood type, advance directives) at the top; medications next; recent vitals as charts; recent symptoms; active diagnoses.
- **Intervention logging** — fast-entry tools for vitals (BP, HR, SpO2, temperature, GCS), drugs administered (dose, route, time), observations, free-text notes.
- **Case timeline** — chronological view of everything happening in this case: arrival on scene, vitals trend, drugs given, handoffs.
- **Handoff** — end-of-call action: select receiving ER / hospital, submit handoff. The receiving station's relay establishes its own grant; the EMS case becomes the predecessor of the new ER case.
- **Offline operation** — ambulance is often in dead zones. OHD Emergency caches everything locally, syncs when connectivity returns.

### OHD Emergency dispatch console

Operator-side console for the EMS station. Web app, run on the station's infrastructure.

Functions:

- **Active case board** — every case currently in flight, with assigned crew, patient location, status.
- **Crew authentication / roster** — manages which paramedics are currently on duty and authorized to use the relay.
- **Audit visibility** — every break-glass initiated, who authorized it, what was accessed, what was written. Operator-side accountability layer (in addition to the patient-side audit OHD records).
- **Reopen-token issuance** — when an auto-closed case needs to be reopened, the dispatcher issues the reopen token to the field crew.
- **Operator-side records** — the EMS station's own records of cases, separate from the patient's OHD record. Stored in the operator's database; subject to the operator's retention policy and regulatory regime.

### OHD Emergency MCP

LLM tools tailored for emergency triage and dispatch. Connects to the operator's relay; sees patient data via active emergency grants.

Tools:

- **Patient brief** — `find_relevant_context_for_complaint(complaint)` — pulls the data slices that matter for the chief complaint (chest pain → recent BP/HR + cardiac meds + cardiac history; possible OD → drugs taken + history + allergies).
- **Vitals trending** — `summarize_vitals(window)`, `flag_abnormal_vitals()`.
- **Medication interactions** — `check_administered_drug(drug_name, dose)` against the patient's current medications and allergies.
- **Handoff briefing** — `draft_handoff_summary` produces a structured handoff to the receiving ER.

Intentionally narrow — emergencies are time-critical, the LLM should be a triage assistant, not an exploratory analytics tool.

### OHD Emergency CLI (`ohd-emergency`)

Mostly for operators / sysadmins / scripts. Issuing certs, managing the relay, querying audit logs, exporting case archives for legal review.

## Operator-side records

OHD Emergency includes a deployable operator-side records layer — the EMS station's database of cases, interventions, personnel actions, billing details. This stays **outside the OHD protocol** but is part of what OHD Emergency provides for a complete lightweight EMS deployment.

The flows:

- Paramedic's tablet records intervention → writes to **patient's OHD via OHDC** (canonical record, owned by patient) AND to **operator's records via the operator's database** (clinical-safety redundancy, regulatory compliance, billing).
- Both records persist independently of each other after the case ends.
- Operator's records are governed by the operator's retention policy and regulatory regime (HIPAA in the US, GDPR in the EU, etc.). Patient's revocation of the OHD grant doesn't retroactively delete the operator's records.

This is consistent with how healthcare records work today and is explicitly in scope for OHD Emergency as a deployable package — but it's NOT in scope for the OHD protocol. Operators using their own EMS database (existing ImageTrend deployment, etc.) can integrate just the OHDC client part of OHD Emergency without using its operator-side records layer.

## Deployment

A typical OHD Emergency deployment for an EMS organization:

1. **Authority cert** — apply to the OHD project (or relevant country CA) for an authority cert. Verification: real org, real roster, regulatory accountability.
2. **OHD Relay in emergency-authority mode** — Docker Compose deployment, fronted by Caddy, holds the authority cert.
3. **Dispatch console** — same infrastructure or alongside.
4. **Operator-side records database** — Postgres (or whatever) for the EMS station's records.
5. **Paramedic tablets** — provisioned with the org's identity (clinic SSO / Okta / etc.) and configured to authenticate to the org's relay.
6. **Personnel onboarding** — added to the org's roster system; their credentials accepted by the relay.

Cost: a Hetzner VPS for the relay + dispatch + records can run a small EMS station's OHD Emergency for tens of euros a month. A larger org runs it on their existing server infrastructure.

## What OHD Emergency does NOT do

These are deliberate scope boundaries:

- **No CAD** — vehicle dispatch, GPS routing, resource allocation. Use a real CAD product alongside.
- **No NEMSIS / HL7 export pipelines** — OHD Emergency speaks OHDC. NEMSIS-compliant export is a separate add-on, deployable alongside if the org needs to report to national registries.
- **No insurance / billing** — operator's existing systems.
- **No personnel scheduling** — operator's HR systems.
- **No imaging viewer** — image attachments referenced but rendered by an external viewer.
- **Not a hospital ER information system** — the receiving ER's tooling is its own concern (OHD Care, an existing EHR, or the hospital's bespoke system).

A full-service EMS deployment runs OHD Emergency *alongside* a CAD system, an NEMSIS reporter, and the operator's HR. OHD Emergency owns the OHDC integration; the rest stay in their existing tools.

## Trust boundary

OHD Emergency holds **authority capability** (its relay's signing cert) and **active grants** for patients during cases. It does not hold the patient's broader data — only what the case grant exposes.

When a case ends:

- Active grant transitions to read-only on the case's frozen span.
- Operator-side records persist under the operator's regulatory regime.
- New cases for the same patient require fresh break-glass.

When a case authority is revoked by the patient mid-case:

- OHDC reads stop immediately.
- Operator's existing records are preserved; the operator's regulatory regime governs what happens to them.
- The case's `revoked_at_ms` is set; audit records the patient revocation.

## Security

- **Authority cert protection** — the relay's authority cert is a high-value secret. Hardware-backed (HSM, TPM, or platform-grade secure storage). Compromise means revoking the cert at the trust root and re-provisioning, which invalidates all the operator's outstanding grants.
- **Roster integrity** — the operator must keep their personnel roster current. Departed paramedics must lose access promptly. Audit logs document who was authenticated when.
- **Operator-side records encryption** — operator's database holds PHI; full encryption at rest expected. Our reference deployment uses Postgres + LUKS / equivalent.
- **LLM exposure (Emergency MCP)** — when emergency MCP feeds patient data to the operator's chosen LLM, that data goes wherever the LLM provider lives. For sensitive deployments, self-hosted models are the conservative choice. Our reference includes a "no PHI to external LLMs" config knob.
- **Tablet device management** — paramedic tablets are mobile, easily lost. Device management (encryption, remote wipe, biometric lock) is the operator's compliance responsibility.

## Open design items

- **NEMSIS / HL7 bridge** — reference operator-side records → NEMSIS export. Not in scope for the core but a clear add-on; useful for operators who must report.
- **Multi-station handoff workflows** — when EMS hands off to an ER that's a different operator. The case's `predecessor_case_id` link is the protocol-level mechanism; the UX of "select the receiving facility" is what we'd want a clean reference for.
- **Bystander permission UX** — how does a passerby with OHD Connect installed know they're forwarding an emergency request? Currently it happens transparently. There's a debate about whether to make it visible (educate the public about the good-Samaritan transport feature) or stay invisible (avoid surfacing every emergency event in the world to every OHD installation).
- **Operator-side records data model** — for the reference deployment, a documented schema is needed. Outside the OHD protocol but part of what OHD Emergency ships.
- **Country-specific CA federation** — long-term, OHD Emergency operators connect to per-country emergency-services trust roots, not just the OHD project's default root. Governance and onboarding processes for country CAs are deferred.
