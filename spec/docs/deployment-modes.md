# Deployment Modes

OHD is a protocol, not a product. Anyone can run an instance — for themselves, for a small group, for an entire patient population. The question for every user is: *where does my OHD instance live?*

The four realistic answers:

| Mode | Where it runs | Who pays | OHD-guaranteed? |
|---|---|---|---|
| **On-device** | Your phone or laptop | nobody (free) | no |
| **Personal storage** | A VPS / NAS you control | you (hardware/hosting cost) | no |
| **Custom provider** | An employer / insurer / clinic / community-run instance | them (or their subsidy) | no |
| **OHD Cloud** | Reference SaaS run by the OHD project | you (subscription) | yes |

All four speak the same protocol. Your data is exportable from any of them and importable into any other. The choice is operational and economic, not a lock-in.

## On-device

Your OHD instance is the SQLite + chunk files inside the OHD Connect mobile/desktop app. No server, no internet dependency.

**Why people choose this**
- **Free.** No hardware to buy, no subscription, no third party.
- **Maximally private.** Your data never leaves the device unless you actively export it.
- **Privacy-maximalist deployments** — useful where any cloud presence is a non-starter (sensitive populations, jurisdictions with weak privacy law, journalists / activists).

**Tradeoffs**
- **Sharing is awkward.** Grant tokens require the device to be reachable when the grantee queries. Your phone is sometimes off, sometimes asleep, sometimes on metered network. A doctor querying your data at 3 a.m. before your visit doesn't get an answer.
- **No continuous-access guarantee.** OHD can't promise the data is reachable when needed. That undermines the "show your doctor" use case for anyone who isn't *actively present* during the visit.
- **You lose the phone, you lose the data.** Backups are your responsibility. Exports must be regular and stored somewhere safe.
- **Storage and compute are bounded** by the device. Years of CGM data + workout HR series eat tens of gigabytes; older phones may struggle.
- **Other apps on the device** can't normally read your OHD instance, but the OS sandbox is the only enforcement.

**When this is the right answer**
- You want to log and analyze your own data and you don't need a doctor / family / researcher to query it asynchronously.
- You're allergic to any cloud presence.
- You're testing OHD before committing to a hosted setup.

## Personal storage (self-hosted)

You rent a small VPS (Hetzner, Linode, OVH, etc.) — or run a NAS at home — and `docker compose up` the OHD reference stack. About €5–10/month on a small VPS, or ~zero ongoing cost on hardware you already own.

**Why people choose this**
- **Full control.** You hold the keys, you choose the jurisdiction, you choose the backup strategy. No third party can be subpoenaed for your data without coming to you.
- **Easy sharing.** The instance has a stable URL (`ohd.your-domain.org`). You hand a grant token to your doctor, they query whenever they need to. Asynchronous, durable, no awkward "is your phone on?" dance.
- **Predictable cost.** Storage + compute on a Hetzner cx23 is more than enough for a single-user lifetime, for a fixed monthly price.
- **No vendor lock-in to OHD itself.** If we ever go away, your VPS keeps running. Reference image stays available; protocol is open.

**Tradeoffs**
- **You operate it.** Patches, backups, monitoring, certificate renewal, restore procedures. Caddy + Docker make this light, but it's not zero.
- **You pay the hardware cost** even when you're not using it.
- **You're responsible for security.** A misconfigured VPS firewall is your problem.
- **OHD project doesn't guarantee uptime** — you're the operator.

**When this is the right answer**
- You're technically comfortable with `ssh` and `docker compose`.
- You want sharing with doctors / family / researchers to *just work*.
- You'd rather pay €5/mo to a VPS provider you already trust than a SaaS subscription.
- You want to run multiple users on one instance (family, small clinic).

## Custom providers

A third party — your employer, your insurer, your clinic, your community health co-op, a national health service — runs an OHD instance and gives you an account on it. Same protocol; just hosted by someone else who has their own reasons for offering it.

**Why people choose this**
- **Free or heavily subsidized.** "Health-data benefit" from your job, included in your insurance plan, run by your clinic as part of patient onboarding.
- **Continuous access.** They run the infrastructure, they keep it up.
- **Often comes with extra UI.** Clinic-side dashboards, integrations with their EHR, payor-side analytics for the user (e.g., "your monthly health summary report").
- **The provider may pre-populate data.** Your clinic's lab pushes results directly. Your insurer pulls them with consent.

**Tradeoffs**
- **The provider has access by definition.** OHD's principles ask them to honor the spirit (no data sale, easy export, no obstruction to leaving), but OHD as a project can't enforce that on third-party operators. You're trusting their policy + jurisdiction.
- **Switching providers requires you to export and import.** That works because the protocol is portable, but it's a real action you have to take.
- **Quality varies.** A community-run instance with one volunteer admin is not equivalent to a clinic with full IT.
- **Their interests may diverge from yours.** An insurance-run instance has incentives that aren't perfectly aligned with the patient's.

**When this is the right answer**
- A trustworthy provider in your life already offers it. Your big employer, your university health service, your forward-thinking clinic.
- You're not a self-hoster, the cost of OHD Cloud isn't appealing, and you've vetted the provider's policies.
- You want them to *also* push data in (lab results, prescriptions, appointment notes) — that's their value-add over you self-hosting.

## OHD Cloud

The reference SaaS, run by the OHD project itself. One operator, well-known policies, professional ops.

**Why people choose this**
- **Zero setup.** Sign up, get an instance URL, point your Connector at it.
- **Always reachable.** Standard SaaS uptime, monitored, replicated.
- **Backups, updates, security patches** — handled.
- **Sharing works the same way as self-hosted** — grant tokens, audit logs, durable URL.
- **OHD's own operator policy is the strongest commitment we'll ever publish.** No data sale, no data mining, no surprise scope changes. Auditable. Spelled out in `02-principles.md`.
- **You can leave any time.** One-click export of everything, importable into any other deployment mode. The point of OHD Cloud isn't to lock you in; it's to be the lowest-friction default for people who would otherwise self-host badly.

**Tradeoffs**
- **It costs money.** Storage and a small amount of compute. Pricing reflects actual cost, not VC-subsidized growth-hacking.
- **You're trusting the OHD project as an operator.** Same as any SaaS: you're betting on the operator's policies + practices over time. We do everything we can to make this trust legible (open-source code, public principles, audit reports), but it's still trust.
- **Single point of operator-failure.** If OHD as an organization stops existing, you're moving. Export-on-demand is what makes this survivable; the same export works against every other deployment mode.

**When this is the right answer**
- You want OHD to "just work" without running a VPS.
- You're willing to pay a few €/month for storage + reachability + backups + clinical-grade compliance.
- No suitable custom provider is available to you.
- You're a small clinic / small business that wants OHD-as-a-service for your patients/employees without operating it yourselves.

## How to choose

A short decision tree:

1. **Does someone you trust already run an OHD instance for you (employer, clinic, insurer)?**
   → Use that. It's the cheapest and probably the highest-quality option.
2. **Are you comfortable with `ssh` + `docker compose`, and do you want maximum control?**
   → Self-host. Hetzner cx23 is €5/mo and runs forever.
3. **Do you want OHD-quality, OHD-supported, no-ops experience, and you're OK paying for it?**
   → OHD Cloud.
4. **Do you flatly refuse any cloud presence, even your own?**
   → On-device. Accept that sharing is awkward.

You can also start in one mode and migrate later. The export format is the same for all four — that's the entire point.

## OHD's commitment, regardless of mode

- **Your data is yours, always.** Every mode supports full export at any time.
- **The protocol is open.** No mode requires proprietary tooling.
- **Migration is supported.** Going from on-device → self-hosted → custom provider → cloud (or any other order) is a documented operation, not a re-onboarding.
- **No mode taxes the others.** Cloud users don't subsidize the protocol; self-hosters don't pay anything to the project.

The point of all four modes existing is the *protocol* survives any one of them disappearing. If OHD Cloud shuts down, self-hosters don't notice. If the on-device mode never ships, cloud and self-host still work. If a custom provider exits the market, their users export and pick another mode.
