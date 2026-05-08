# 02 — Principles & Licensing Philosophy

These are the non-negotiables. Every design decision should be checkable against these.

## Core principles

### 1. Data belongs to the person

The data describes a person. It is theirs. Not the hospital's, not the insurance company's, not the OHD project's, not the Connector developer's. If the data is about you, you own it.

**Consequences:**
- A user can always export their complete data.
- A user can always delete their data (with appropriate retention for audit logs).
- A user can always move to a different OHD provider.
- A user can always revoke access they previously granted.
- A provider hosting a user's OHD instance must facilitate these rights, not obstruct them.

### 2. No central identity

The core OHD protocol does not store personal identity. Users are identified by opaque UUIDs, authenticated via external OIDC providers they choose.

**Consequences:**
- A data breach of the core OHD database leaks "user 12847392 has these glucose readings" — not "John Smith has these glucose readings."
- Users can use pseudonymous OIDC providers if they want maximum privacy.
- Providers hosting OHD instances may of course collect identity for their own purposes (a hospital has your real name) — but that identity lives in *their* layer, not in the OHD protocol itself.

### 3. Access is explicit, scoped, and audited

Every read of another person's data requires explicit, scoped, time-limited authorization, and is logged.

**Consequences:**
- Default permission is zero. No one sees your data without a grant.
- Grants are narrow by default (this event type, this time range, this duration).
- Every query is logged with what was asked, who asked, and what was returned.
- Users can inspect their audit log anytime.
- Emergency access ("break glass") is supported but always audited.

### 4. Portability is lossless and standardized

Data can move between OHD instances without loss.

**Consequences:**
- The export format is a standard, not an implementation detail.
- Any OHD instance MUST be able to export its data in the standard format.
- Any OHD instance MUST be able to import the standard format, or clearly document what it can't represent.
- Extensions to the standard format must either be reversible (round-trippable) or flagged as lossy.
- A provider that implements a proprietary extension should, where it benefits everyone, contribute it back to the protocol.

### 5. Comprehensive representation

Any health-relevant data a person might want to record should have a place in OHD.

**Consequences:**
- Biometrics (instant and continuous) are supported.
- Events with duration (meals, exercise, sleep) are supported.
- Subjective data (symptoms, mood, notes) is supported.
- Medical records (diagnoses, prescriptions, lab results) are supported.
- Hospital-generated data (surgery notes, imaging metadata, operation logs) is supported.
- If someone has a legitimate use case that OHD can't represent, that's a protocol gap we should close.

### 6. Open source, freely redistributable

The reference implementation is open source. Forks, commercial deployments, competing implementations are all welcome.

**Consequences:**
- Anyone can run an OHD instance for themselves or for others.
- Anyone can build an OHDC consumer (personal app, clinical app, integration) against the protocol.
- Hospitals don't owe us licensing fees.
- Competition between OHD providers benefits users (it's always easy to switch).

## Licensing philosophy

The licensing is designed to achieve: **maximum adoption with strong user-data protections**.

### Core repository license

The OHD core (server, Android app, web apps, CLI tools, specs) will be released under a permissive open-source license — likely **Apache 2.0** or **MIT**. This is deliberately permissive to encourage adoption.

### What we ask of implementers (not legally required, strongly recommended)

If you run or fork OHD, we ask that you follow the spirit of the project:

1. **Honor user data ownership.**
   - Users can export their data from your instance at any time.
   - Users can delete their data from your instance at any time (subject to legal retention requirements for audit logs only).
   - Never use a user's data for purposes they didn't explicitly consent to.

2. **Support portability.**
   - Your export must be in the standard format defined by the protocol.
   - Your import must accept exports from the reference implementation.
   - If you've extended the data model, declare those extensions in the export metadata so they can be either preserved or gracefully stripped by other instances.
   - **Exporting incomplete data is preferable to blocking export.** If you can't round-trip some extension, export what you can and flag what you couldn't.

3. **Contribute improvements back where it helps everyone.**
   - Bug fixes: please submit upstream.
   - Security improvements: please submit upstream.
   - New data types that are broadly useful: please submit upstream.
   - Private extensions that only make sense for your deployment: keep them private, that's fine.
   - This is the open-source norm — we're just stating it explicitly.

4. **Respect security and compliance standards.**
   - If you deploy to users in jurisdictions with health-data regulations (HIPAA, GDPR, etc.), comply with them. That's your responsibility as the operator, not ours.
   - Encrypt data at rest where the threat model requires it.
   - Use TLS. Always.
   - Segregate user data. A breach of one user shouldn't cascade.

5. **Don't misrepresent yourself as the official project.**
   - If you fork, make it clear it's a fork.
   - Don't use the OHD name or logo in a way that implies endorsement.

### What we commit to

1. **We will never sell user data.** The reference SaaS (if we run one) will charge for storage and compute, not for the data itself. This applies to the non-profit OHD organization; third-party deployments can do what they want subject to their own jurisdictions and their users' consent.

2. **We will maintain the export format.** Once a format version is released, we commit to long-term import compatibility — users who export from OHD today will be able to import into OHD ten years from now.

3. **We will publish all protocol changes openly.** No proprietary extensions in the core. If we add it to the reference implementation, it's in the spec.

4. **We will maintain a reference implementation forever, or until it's clearly no longer useful.** We won't abandon users mid-deployment. If the project is ever shut down, the final release will be a clean, deployable Docker image with export functionality intact.

### What we explicitly do not require

- **You don't have to open-source your changes.** We'd like you to, but we won't force you.
- **You don't have to host users' data for free.** Charge for storage and compute if you want.
- **You don't have to use our SaaS.** Roll your own, that's fine.

### Attribution

A minimal attribution requirement ("based on Open Health Data, ohd.org") will be part of the license, but won't be onerous — similar to Apache 2.0's notice requirement.

## Licensing decision: dual-licensed Apache-2.0 OR MIT

The repository is **dual-licensed under your choice of Apache 2.0 or MIT** — the standard Rust-ecosystem convention. Files: [`LICENSE`](../LICENSE) (overview), [`LICENSE-APACHE`](../LICENSE-APACHE), [`LICENSE-MIT`](../LICENSE-MIT), [`NOTICE`](../NOTICE). The "spirit of the project" asks (this section's consequences) live in [`SPIRIT.md`](../SPIRIT.md) as non-binding requests, not license clauses.

Why the pair:

- **MIT** for projects and downstream consumers that want minimal license text and the broadest compatibility (including with GPLv2 codebases).
- **Apache 2.0** for the explicit patent grant and patent-retaliation clause — health tech is a patent-active space, and the Apache half gives downstream users a real shield.

AGPL was considered and rejected: it would legally require open-sourcing modifications, which sounds aligned but in practice would block hospital and enterprise deployments that the project's reach depends on. The non-binding `SPIRIT.md` asks are the right shape — strong norms, no enforcement teeth.

## A note on the tone of this document

These principles sound lofty. They're not meant to. They're meant to be practical: every time we make a design decision, we should be able to point at a principle and say "this follows from that."

If we ever make a decision that violates one of these principles, we should write it down, explain why, and accept that we've made the project slightly worse in that dimension. And we should probably revisit it.
