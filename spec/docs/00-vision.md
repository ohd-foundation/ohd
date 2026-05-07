# 00 — Vision

## The problem

Health data is fragmented, siloed, and owned by everyone except the person it describes.

- **Your smartwatch** has your heart rate and step count.
- **Your CGM app** has your glucose readings.
- **Your hospital's EHR** has your diagnoses and prescriptions.
- **Your GP's practice software** has your consultation notes.
- **Your nutrition app** has what you ate last Tuesday.
- **You** have none of it in one place.

When you go to a new doctor, you start over. When you're in an ambulance, the paramedics don't know what medications you take. When a specialist wants to correlate your glucose with your sleep, they can't — the data lives in different companies' databases. When a researcher wants to study how a new drug affects real people in real life, they have to run a clinical trial because there's no way to ask 10,000 willing patients directly.

Meanwhile, platforms like **Google Health Connect** and **Apple HealthKit** have solved the "aggregate data from wearables" problem beautifully — but they stop there. They don't let doctors read your data. They don't support food logging or medication tracking. They don't have audit trails. They aren't a record you can show a specialist.

The infrastructure exists. The last mile is missing.

## What OHD is

OHD is the last mile.

It's a **protocol** that defines how personal health data is structured, stored, queried, and exchanged. It's also a **reference implementation** — a running backend, an Android app, a web dashboard, MCP servers — so the protocol isn't just theoretical.

The protocol has three properties that matter:

1. **You own the data.** The database is yours. You decide where it lives (your phone, your server, our SaaS, your hospital's infrastructure). You decide who can read it. You can revoke access instantly. You can export it and move to a different provider losslessly.

2. **Anyone can implement it.** It's open source. Hospitals, insurers, healthcare startups, individuals, national health services — anyone can run an OHD instance. The goal is an ecosystem, not a company.

3. **It's comprehensive by design.** Biometrics (continuous and event-based), meals (with timing and duration), medications (prescribed, taken, skipped), symptoms, exercise, doctor notes, hospital records, lab results, imaging references — all in one model. If a use case can't be expressed in OHD, the protocol is incomplete and needs fixing.

## Who it's for

- **Individuals** who want to actually understand their own health, not just trust a fragmented system.
- **Doctors** who want real data about their patients instead of five-minute self-reports.
- **Paramedics and emergency responders** who need to know a patient's medications and conditions *now*.
- **Researchers** who want to study real-world health at scale without bureaucratic data brokers.
- **Hospitals** who want to hand patients a portable record instead of a printout nobody reads.
- **Insurance companies** (with user consent) who want to offer better premiums based on actual behavior.
- **Healthcare systems** in countries that want to modernize without vendor lock-in.

## What OHD is not

- **Not a diagnostic tool.** OHD stores data. It doesn't tell you what's wrong with you. (OHD Care or other clinical apps built on top might, using LLMs — but that's a separate layer.)
- **Not a replacement for doctors' EHRs.** Doctors keep their own records for legal reasons. OHD is the patient-owned parallel record that *supplements* the EHR, not replaces it.
- **Not a commercial data broker.** The core project never sells data. If a user wants to sell their own data to researchers, that's their choice, mediated by a researcher-portal OHDC consumer — but the infrastructure doesn't do it on their behalf.
- **Not a walled garden.** We want hospitals, other startups, national health services to fork it, extend it, compete with us. That's a feature.

## Why now

Three things make this possible in a way it wasn't five years ago:

1. **Standardized wearable data.** Google Health Connect and Apple HealthKit normalize sensor data. OHD doesn't need to integrate with a hundred different device APIs — it integrates with the two platforms that already did that work.

2. **LLMs + MCP.** Querying health data used to require dashboards, SQL, and technical skill. Now a doctor can ask "how did this patient's glucose respond to metformin over the last three months?" and an LLM can answer via the Care MCP. Clinical workflows become useful *immediately* instead of after years of UI work.

3. **Docker + cheap VMs.** A self-hosted OHD instance costs pennies per month. A hospital-hosted one costs less than their paper records do. Decentralization is finally cheaper than centralization.

## The vision, stated plainly

A world where your health data lives with you, travels with you, and is understood by whoever you choose to share it with — including yourself. Where your doctor spends the consultation looking at real data instead of asking you to remember what you ate last week. Where researchers can accelerate studies by orders of magnitude because volunteers can share real-world data in one click. Where a paramedic can know you're diabetic before they load you into the ambulance.

And all of it built on a protocol anyone can run, fork, or improve.

That's what OHD is.
