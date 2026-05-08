# Spirit of the Project

> Non-binding requests for everyone who runs, forks, integrates with, or builds on top of OHD. Not license clauses. We can't enforce these and we won't try. They're written down here because they're load-bearing for what OHD is trying to be — and forks that violate them, while legally fine, are working against the project's purpose.

If you find yourself reading this and disagreeing with one of these asks, that's a signal to think carefully about whether OHD is the right base for your project. There are perfectly legitimate reasons to fork in directions that don't match the project's spirit; we just want you to do that with eyes open.

## 1. Honor user data ownership

The data describes a person. It is theirs. Your role as an operator, integrator, or app developer is custodial.

- **Export anytime, no friction.** A user must be able to export their complete data from your instance whenever they want. The export must be in the standard OHD format. No paywall, no waiting period, no "ask support."
- **Delete anytime, no friction.** Same. A user must be able to delete their data from your instance — leaving only the minimum audit retained for legal traceability.
- **Move anytime.** A user who exports from your instance must be able to import into any other OHD-compatible instance, losslessly. Your job is to make that work, not to make it hard.
- **Don't use a user's data for purposes they didn't consent to.** No data sale, no analytics-on-individuals, no model training, no surprise scope changes. If you want to do something new with the data, ask first, with a clear opt-in, and respect "no."

## 2. Support portability

The whole point of an open protocol is that you're not locked in. If you fork or extend OHD:

- **Your export must be in the standard format.** You can extend the format, but the standard parts must round-trip cleanly.
- **Your import must accept exports from the reference implementation.** If something can't round-trip (you've stripped a feature, or your fork has a different schema), declare what you couldn't preserve and pass it through as opaque metadata. **Exporting incomplete data is preferable to blocking export.**
- **Document your extensions.** Other implementations should be able to look at your export and understand what's standard vs. what's yours.

## 3. Contribute improvements back where it helps everyone

Open-source norm, stated explicitly:

- **Bug fixes**: please submit upstream. They help everyone.
- **Security improvements**: please submit upstream. Especially these.
- **New data types and channel-registry entries that are broadly useful**: please submit upstream so they become standard.
- **Private extensions that only make sense for your deployment**: keep them private. That's fine — that's what custom namespaces are for.
- **Forks that improve the core**: please submit upstream first; if it's accepted, your downstream burden shrinks. If it's rejected for principled reasons, your fork is yours; that's fine.

This is **politeness**, not obligation. The license lets you fork without contributing. We're just noting what makes the project better for everyone.

## 4. Respect security and compliance standards

If you deploy OHD to users in jurisdictions with health-data regulations (HIPAA, GDPR, the various national equivalents), comply with them. That's your responsibility as the operator, not ours.

- **Encrypt data at rest** where the threat model requires it (it usually does).
- **Use TLS 1.3 always.** No exceptions.
- **Segregate user data.** A breach of one user shouldn't cascade.
- **Maintain the audit log.** Don't truncate or alter it without legal grounds.
- **Have an incident-response plan.** Know what you'll do when (not if) you get breached.

## 5. Don't misrepresent yourself as the official project

The trademark notice in [LICENSE](LICENSE) covers the legal side; the spirit is:

- **If you fork, make it clear it's a fork.** Different name, different visual identity, different domain.
- **Don't use "OHD" or "Open Health Data" as your product name.** "Based on OHD" is fine; "OHD by AcmeCorp" is not.
- **Don't claim certifications, endorsements, or affiliations you don't have.** The project doesn't grant compliance certifications; nobody else can claim them on the project's behalf.

## What the project commits to in return

The project (and the OHD non-profit / commercial-vehicle / contributors-collective, whatever ends up holding the trademark and IP) commits to:

1. **Never sell user data.** The reference SaaS, if there is one, charges for storage and compute, not for the data itself.
2. **Maintain the export format long-term.** Once a format version is released, exports from that version will be importable by future versions for at least 10 years.
3. **Publish all protocol changes openly.** No proprietary extensions in the core. If we add it to the reference implementation, it's in the spec.
4. **Maintain a reference implementation as long as it's useful.** If the project is ever shut down, the final release will be a clean, deployable image with full export functionality intact.
5. **Defend the trademark.** Not aggressively, but enough to prevent bad-faith uses that would harm users.

## Where these come from

These principles are derived from [`docs/02-principles.md`](docs/02-principles.md), which is the canonical statement of the project's values. The split is intentional:

- `02-principles.md` is the **why** — the rationale, the philosophy, the design constraints these create.
- `SPIRIT.md` (this file) is the **what we ask of you** — concrete, actionable, in plain language for someone who's about to fork or deploy.

If they ever conflict, `02-principles.md` is the source of truth.
