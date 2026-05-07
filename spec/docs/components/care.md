# Component: OHD Care

> A reference, real-and-usable, lightweight EHR-shaped consumer of the OHDC protocol. Designed for OHD-data-centric clinical workflows.

## What OHD Care is

OHD Care is the canonical professional-side application of OHD: a multi-patient clinical app that healthcare professionals (doctors, specialists, nurses, paramedics, therapists) use to access and contribute to a patient's OHD. It speaks OHDC like every other consumer; what makes it specific is the clinical workflow shape — patient roster, encounter context, write-with-approval, read-driven visit prep.

OHD Care is **not**:

- Trying to replace Epic / Cerner / large enterprise EHRs.
- A full clinical workflow management platform (no billing, no scheduling, no claims, no thousand-other-things commercial EHRs handle).
- A demo. It's intended to be deployed and used.

OHD Care **is**:

- A real, open-source, lightweight clinical app — usable in a small practice, a home-care service, an ambulance crew's tablet, a specialist's office, a clinical trial site, a direct-pay clinic that never wanted Epic, or alongside an existing EHR that doesn't speak OHDC yet.
- The reference for "this is how an EHR-shaped consumer is built on OHD" — operators, developers, and integrators look at OHD Care to understand the pattern.
- Focused: the OHD-data-centric clinical workflow, done well, deployable in an afternoon.

## The value story (what changes in a visit)

A patient comes in feeling sick. They've been logging temperature at home via OHD Connect. They have an active grant for their family doctor.

**Without OHD**: doctor asks "have you had a fever? when? how high? did you take anything?" — patient guesses. Recall is incomplete; trends are unavailable.

**With OHD Care**: doctor opens the patient's record. Care has already pulled:

- Temperature chart for the last 72 hours, with annotations for medications taken.
- Recent symptom logs ("fatigue", "cough", "headache") with timestamps.
- Hydration / fluid intake.
- Any related family contacts logged ("partner has flu — confirmed").

The doctor uses the saved minutes to actually treat. Then they add findings (assessment, medication, follow-up note) which submit through OHD Care's write-with-approval flow — the patient confirms what enters their record on their phone, immediately or later.

The whole thing happens at the speed of "open patient and read", not "interrogate and reconstruct." That's the pattern OHD Care exists to demonstrate.

## Forms

### OHD Care (web app)

The primary form. SPA; deployed by the operator (clinic, hospital, mobile-care service, individual practitioner) against their own OHD Storage instances or against patients' chosen OHD Storage instances.

Multi-patient roster, per-patient view, chart builder, write-with-approval submissions, audit transparency, LLM-assisted brief.

Layout (rough):

- **Roster** — every patient who has granted the operator access. Status indicators (last visit, recent flags, current medications summary).
- **Per-patient view**:
  - Header: identifying details (operator-side, optional), active grant scope (so the operator knows what they can see), grant expiry.
  - Tabs: Timeline, Vitals, Medications, Symptoms, Foods, Labs, Imaging, Notes — each driven by query against the active patient's grant.
  - Visit panel: previous-visit summary, assessment input, write-back (queues for patient approval per grant policy).
- **Chart builder** — pick channels, pick range, render. Saved views per operator.
- **Audit** — what queries the operator has made on this patient, surfaced for transparency. The patient sees the same audit on their side.

### OHD Care MCP (`ohd-care-mcp`)

LLM-assisted clinical workflow. The distinctive feature is **active-patient context** — the LLM session holds N grants and a `switch_patient(label)` tool changes which grant is in scope for subsequent calls.

Tools:

- **Patient management** (multi-grant routing):
  - `list_patients` — all patients the operator has grants for, with brief status.
  - `switch_patient(label)` — set the active patient's grant as scope for subsequent calls.
  - `current_patient` — diagnostic.
- **Read tools** (against active patient, gated by their grant's read scope):
  - `query_events`, `query_latest`, `summarize`, `correlate`, `find_patterns`, `chart`, `get_medications_taken`, `get_food_log`.
- **Write-with-approval tools** (against active patient, gated by their grant's write scope; queue per the grant's `approval_mode` policy):
  - `submit_lab_result`, `submit_measurement`, `submit_observation`, `submit_clinical_note`, `submit_prescription`, `submit_referral`.
- **Workflow tools**:
  - `draft_visit_summary` — produces a patient-readable summary the operator reviews and submits.
  - `compare_to_previous_visit` — narrative diff.
  - `find_relevant_context_for_complaint(complaint)` — pulls the data slices likely to matter for the chief complaint.

A doctor session: "Switch to Alice. She's here with flu-like symptoms" → `switch_patient("Alice")` → `find_relevant_context_for_complaint("flu-like symptoms")` → MCP returns temp series, recent meds, recent symptoms, fluid intake → doctor reviews → adds findings → `submit_clinical_note(...)` → goes to Alice's pending queue → Alice approves on her phone.

The MCP is what makes OHD Care viable as a clinical tool — without it, the operator types and clicks. With it, they speak naturally and the structured data flows.

### OHD Care CLI (`ohd-care`)

Terminal interface for scripts and operators who prefer the keyboard:

```
$ ohd-care patients
$ ohd-care use alice
$ ohd-care temperature --last-72h
$ ohd-care submit observation --type=respiratory_rate --value=18
$ ohd-care submit clinical-note --about="visit 2026-05-07" < notes.txt
$ ohd-care pending list  # see what's queued for patient approval
```

Same auth model — uses grant tokens; switches active grant via `use <label>`.

## Workflow features

### Visit prep

When the operator opens a patient (UI) or runs `find_relevant_context_for_complaint(...)` (MCP), Care:

1. Pulls relevant time series (most recent + trend).
2. Pulls recent medication adherence.
3. Pulls recent related symptoms / food / activity.
4. Pulls the previous-visit summary, if any.
5. Surfaces flags: "BP trending up over last month", "missed 4 of last 14 doses", "new symptom in last 48h".

This renders as a one-screen brief at the top of the patient view.

### Write submission

When the operator adds clinical content:

- Each submission is a typed event with proper channels (`clinical_note`, `lab_result`, `medication_prescribed`, `referral`, etc.).
- Submissions go through OHDC `put_events` against the active patient's grant.
- The grant's `approval_mode` policy determines what happens:
  - `always` (default for new grants): every submission queues for patient review.
  - `auto_for_event_types`: pre-authorized types auto-commit; others queue. Established relationships use this for routine writes (`lab_result`, `clinical_note`) while routing high-stakes writes (`prescription`) through review.
  - `never_required`: all submissions auto-commit (used for trusted long-term grants and emergency / break-glass).
- Submitted events appear in OHD Care's "pending" status until the patient approves (or the policy auto-commits).

### Multi-patient context safety

To avoid the "wrong patient" failure mode:

- The active patient is **prominently displayed** in every UI and MCP context.
- A patient switch requires explicit operator action (no automatic switching from search or LLM intent).
- Submission tools include a confirmation step showing "submitting to Alice — confirm?" before it goes through.
- Every cross-patient action is audited so wrong-patient writes are traceable.

### Operator identity

OHD Care users (operators) authenticate to the Care app itself via the operator's OIDC (their clinic SSO, their personal Google, etc.). The operator's identity is bound to the active grant — every audit row records both the `grant_id` and the operator's identity (in operator-side records, not in OHD's protocol). This protects the patient: even if a clinic's staff turns over, the audit trail shows which person did what.

## Deployment

OHD Care is deployable in many shapes:

- **Single-practitioner**: Docker Compose on a small VPS. The practitioner is the operator and the IT team.
- **Small clinic**: same, with multiple operator accounts via the clinic's OIDC.
- **Hospital department**: deployed in the hospital's infrastructure; integrates with the hospital's identity system; coexists with the main EHR (Epic / Cerner) which doesn't speak OHDC.
- **Mobile / ambulance**: operator app on a tablet, with break-glass grants pre-authorized on patient OHDs.
- **Clinical trial site**: per-study deployment; participants grant the trial's CRO with study-scoped access.
- **Direct-pay / boutique practice**: primary clinical tool, no main EHR.

The deployment story is intentionally lightweight — "stand up Care in an afternoon, integrate with patient OHDs immediately, run a clinic on it." Comparable to spinning up Bitwarden or Nextcloud.

## What OHD Care does NOT do

These are deliberate scope boundaries, not deferred phases:

- **No billing / coding / claims** — that's between the operator and their billing system.
- **No scheduling / calendar** — that's the operator's existing scheduler.
- **No HL7 / FHIR mapping suite** — Care speaks OHDC. Bridges to / from FHIR are separate components.
- **No insurance / payer integration**.
- **No DICOM imaging viewer** — image attachments are referenced but rendered by an external viewer the operator picks.
- **No prescription delivery to pharmacies** — Care submits a `medication_prescribed` event into the patient's OHD; integration with pharmacy systems is an OHDC consumer Care doesn't own.

A practice that needs these either:

- runs their existing system alongside Care (Care for OHD-data-centric clinical workflow, the other system for billing / scheduling / etc.), or
- builds the missing piece as a separate component speaking OHDC.

## Trust boundary

OHD Care holds **grants**, not the patient's data. When a patient revokes a grant, the local Care cache for that patient becomes the operator's HIPAA / GDPR responsibility (snapshot from a moment when access was authorized; retained per the operator's posture).

OHD Care doesn't reach into the patient's storage; it speaks OHDC. The patient sees every Care query in their personal-side audit log, including what was silently filtered out by their grant rules. The patient's revocation is immediate; no cached secret persists at the operator's side beyond the grant's life.

## Security

- **Operator authentication**: OIDC against the operator's identity provider (clinic SSO, Google Workspace, Okta, hospital ADFS, etc.).
- **Patient grant tokens**: stored in operator-side encrypted storage. Lost-laptop scenario is the operator's responsibility (device management, encryption at rest, remote wipe).
- **Multi-patient isolation**: every operation runs against the active patient's grant; cross-patient operations require explicit re-context. No "patient roster query that returns multiple patients' data".
- **LLM exposure**: Care MCP feeds patient data to the operator's chosen LLM. For sensitive deployments, self-hosted models keep PHI in-house. Care exposes a configuration knob for "no PHI to external LLMs"; deployments running it on cloud LLMs accept the tradeoff for their patient population.
- **Source signing for clinical writes**: each operator's submissions are signed with the operator's identity (clinic-level + individual). The patient sees signed-by-X on each pending review. Forgery requires operator key compromise, which the operator controls.

## Open design items

- **Operator-side caching policy**. Care caches for performance (visit prep is multi-query), but cache lifetime + revocation propagation needs explicit deployment guidance.
- **Cross-patient features**. Population-level queries ("show me adherence trends across my diabetic patients") have value but require a different scope model (cohort grants? aggregation-only? per-patient filtering done client-side?). Out of scope for now.
- **Operator-to-operator handoff**. When a patient is referred from one provider to another, both temporarily hold grants. Needs a clean "warm handoff" UX.
- **Break-glass UX**. Emergency / paramedic flow for unconscious patients with pre-issued emergency grants on the patient's lock screen / wristband as a QR. Care needs explicit support for this entry path.
- **Localization**. Care is a clinical app; UI language and clinical terminology need locale-aware versions.
