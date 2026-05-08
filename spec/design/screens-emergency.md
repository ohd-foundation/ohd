# Screens — Emergency / Break-Glass UX

> Designer-handoff doc. Describes the screens, dialogs, settings, and state transitions for the emergency feature in OHD Connect (patient side) and OHD Emergency (responder side). Aesthetic guidance follows `ux-design.md`. Take this as a brief; the .pen files are where the actual designs land.

---

## OHD Connect — patient side

### Settings tab: "Emergency / Break-glass"

A new top-level settings tab. Single page; sections grouped vertically.

**Section: Feature toggle**

- **Heading**: "Emergency access"
- **Subhead**: "Let first responders see basic information about you in a medical emergency."
- **Toggle**: Enable / Disable. Default: Disabled (opt-in).
- When enabled, sub-sections become editable. When disabled, they're greyed out.
- **Footnote**: "When enabled, your phone broadcasts a low-power Bluetooth signal so nearby emergency responders can find your OHD record. They cannot see anything until you (or a timeout) approves."

**Section: Discovery**

- **Bluetooth beacon**: toggle. Default: On (when feature is enabled).
- Subtext: "Broadcasts an opaque ID. No health information leaves your phone via Bluetooth — the beacon only signals 'OHD installed here.' Battery cost is minimal."

**Section: Approval timing**

- **Approval timeout** — slider 10s to 300s, default 30s.
- Subtext: "When a first responder requests emergency access, you have this long to Approve or Reject. After the timeout, the action below applies automatically."

- **If you don't respond before timeout** — radio:
  - **Allow access** (default). Subtext: *"Better for unconscious users. The responder gets your basic emergency info if you can't react."*
  - **Refuse access**. Subtext: *"Better against malicious requests when you're nearby and unaware. Unconscious-you can't grant access this way."*

**Section: Lock-screen behaviour**

- **Approval dialog visibility**:
  - Full dialog above lock screen (default). Subtext: "Recommended for emergencies. Anyone who can pick up your phone can see and approve the dialog."
  - Show only basic info on lock screen. Subtext: "Hides the responder's name and request details until you unlock. Trades emergency convenience for shoulder-surfer protection."

**Section: What responders see**

- **History window** — radio: 0h / 3h / 12h / 24h. Default: 24h.
- Subtext: "How much recent vital-signs history they can see. Even with 0h, they always get current values."

- **Per-channel toggles** — list of channels in your emergency profile. Each row: channel name, toggle, subtext (e.g. "Glucose readings — important for diabetic emergencies"). Defaults follow the standard emergency profile: allergies, active medications, blood type, advance directives, recent vitals (HR, BP, SpO2, temperature, glucose), current diagnoses.
- "Add channel" button at bottom — picks from registered channels not currently in the profile.

- **Sensitivity classes** — toggles for each class. Defaults:
  - General: ON
  - Mental health: OFF (toggle to ON)
  - Substance use: OFF
  - Sexual health: OFF
  - Reproductive: OFF (note: "Some emergencies need reproductive context — consider enabling this if you're pregnant or have body-anatomy concerns")

**Section: Location**

- **Share location** — toggle. Default: Off.
- Subtext: "If enabled, your phone shares its current GPS coordinates with the responding emergency authority when access is granted. Useful for ambulance dispatch when you can't say where you are."

**Section: Trusted authorities**

- **List** of trusted authority roots. Each row: authority name, country / scope, "Manage" button.
- Default: "OHD Project (default root)" — non-removable explanation, plus any per-locale roots auto-added.
- "Add a trust root" button — paste a cert; verify; confirm; add.

**Section: Advanced**

- **Bystander proxy role** — toggle. Default: On.
- Subtext: "Your phone helps forward emergency requests for nearby OHD users who don't have internet. Your phone never sees their data — it just relays encrypted bytes. Disable if you want to opt out of this Good Samaritan behaviour."

- **Reset emergency profile to defaults** button.
- **Disable emergency feature** button (mirrors the top toggle for clarity).

### Cases tab

A top-level tab/section in the personal app's main navigation, visible whenever there are active or recent cases.

**State: No active or recent cases**

- Empty state with explainer: "When emergency responders or healthcare providers access your OHD via cases, you'll see them here."

**State: One or more active cases**

- Prominent card per active case at top:
  - Authority name (e.g. "EMS Prague Region")
  - Started: relative time ("2 hours ago")
  - Started badge (e.g. "Auto-granted via timeout" — if applicable, in distinct color)
  - Tap into details
  - Actions: [Force close] [View audit]
- Below: closed-cases history, sorted recent first.

**State: Case detail view**

- Header: case label, type badge, started/ended times, current authority.
- Tabs:
  - **Timeline** — chronological events recorded during the case
  - **Authorities** — current and predecessor authorities; handoff chain; audit per authority
  - **Audit** — every read and write under this case's grant; auto-granted entries flagged
  - **Settings** — force close, issue retrospective grant, dispute a handoff
- Auto-granted indicator at the top if the original break-glass was a timeout-default-allow.

### Emergency dialog (above lock screen)

Triggered when a verified emergency-access request arrives.

**Layout** (full-screen modal, above lock):

- Top: OHD logo (large, centered).
- Below logo: "Emergency access requested" (bold).
- Authority name, large: "EMS Prague Region" (verified-badge icon).
- Authority's purpose / reason (if provided): "Medical emergency — driving to your location."
- Countdown timer: large, central, animated.
- Timer subtext: "Granting in 30s if no response" (or "Refusing" depending on user setting).
- Two buttons, side by side:
  - **Approve** (large, primary color — red per `ux-design.md` palette)
  - **Reject** (outlined / secondary)
- Optional "Decide later" small text link (re-shows in 5s if dismissed).

**Behaviour**:

- Vibrates and plays a distinct alert tone. Continues until user acts or timeout.
- Cannot be dismissed by Home button or Power button — phone explicitly rejects screen-off until the user acts. (Edge case: if the phone is unconscious / unattended, the timeout resolves naturally.)
- After resolution, dialog disappears; phone returns to lock screen.
- Result is logged in audit; user can review later.

**Lock-screen-basic-only mode**:

If the user has opted into "show only basic info on lock screen", the above-lock dialog shows:

- OHD logo
- "Emergency access requested — unlock to see details"
- Countdown timer
- Buttons: [Unlock to review] [Reject]

The full details (authority name, purpose) require unlock. The Approve action requires unlock first. The Reject action is available unauth. The Allow-on-timeout default still applies if the user doesn't unlock and doesn't reject.

### Notification (after access fires)

When access is granted (interactive or auto), a follow-up notification fires:

- Title: "Emergency access granted"
- Body: "EMS Prague Region accessed your data at 14:23. Tap to review."
- Tap → opens the case detail view.

### Activation paths beyond BLE

The emergency feature also responds to:

- **NFC tap** — patient phone touches responder's device. Same dialog, same flow.
- **QR scan from patient's phone** — responder scans a QR that the patient pre-generated and has on their lock screen. Triggers the same dialog flow on the patient's phone.
- **Remote (no patient-side phone reachability)** — bystander chain handles this transparently. The patient's phone shows the dialog when the request reaches it; if it can't reach (phone off, dead battery), no dialog and no access.

---

## OHD Emergency — responder side

### Patient discovery screen (paramedic tablet)

**Layout**:

- Header: operator label ("EMS Prague Region — Crew 42 — Officer Novák"), connection status (relay reachable / unreachable), GPS-on indicator.
- "Scan for nearby OHD users" big primary button. Pressing it scans BLE for ~10s.
- Result list: each row = one detected OHD beacon, with signal strength, time-since-discovered, and an action button "Request access".
- Manual entry option: "Enter case ID / patient ID" for cases where BLE failed.

**Behaviour**:

- Tapping "Request access" sends the signed emergency request to the patient's phone. The tablet shows "Waiting for patient response... 28s" with the same countdown the patient sees.
- On Approve: data flows; patient view opens.
- On Reject: shows "Request rejected". Crew chooses next action (retry / fall back to verbal / escalate to dispatch).
- On timeout-default-allow (patient setting): tablet receives data, with a UI flag "Auto-granted — patient did not respond within timeout" (so the responder knows this wasn't an active approval).

### Patient view (paramedic tablet)

**Layout** — paramedic-optimized; large fonts, single-column, scrollable.

- Top banner: patient identifier (label), authority status ("EMS Prague Region", "Granted at 14:23 — auto"), case ID, time elapsed.
- Critical info card (red accent): allergies, blood type, advance directives.
- Active medications: list with name, dose, last-taken time.
- Recent vitals: last reading per channel, with mini sparkline of the past N hours (per the patient's history-window setting).
- Active diagnoses / conditions.
- Recent symptoms / observations.
- Bottom action bar: [Log vitals] [Administer drug] [Add observation] [End case / Handoff].

### Intervention logging

Quick-entry UIs for the typical EMS interventions:

- **Vitals** — large numpad-friendly inputs for HR, BP, SpO2, GCS, temp. Auto-tagged with current case_id.
- **Drugs** — pick from a curated list (or search), enter dose / route / time. Auto-tagged with current case_id.
- **Observations** — short structured form (chief complaint, level of consciousness, skin color, etc.) + free text.
- All submissions go through OHDC `put_events` against the active emergency grant; case_id stamped automatically.

### Handoff

When end-of-call:

- Tap [End case / Handoff]
- Select receiving facility (autocomplete from operator's typical destinations + manual entry).
- Optional handoff summary text (the MCP can draft this).
- Confirm.
- Backend: opens new case at the receiving facility's authority (with current case as predecessor), closes current case, transitions current grant to read-only.
- Tablet UI returns to dispatch / next call.

### Dispatch console (operator-side web app)

Browser, run on station's infrastructure.

**Layout**:

- Active cases panel (left): list of cases currently in flight, with crew assigned, patient location, status, time-elapsed.
- Selected case detail (right): timeline of interventions, audit of accesses, current crew member, ability to issue reopen tokens or escalate.
- Crew status panel: who is on duty, who is currently in a case, who is available.
- Audit log (filterable): every break-glass initiated, who, when, against which patient.
- Operator-side records access: read the operator's database of incident records (for billing, follow-up, legal).

---

## State diagram (high-level)

```
[Patient phone — feature enabled]
     │
     ├── (BLE beacon broadcasts)
     │
     ▼
[Responder discovers via BLE / NFC / QR / via bystander-relay]
     │
     ▼
[Responder sends signed request via station relay]
     │
     ▼
[Patient phone verifies cert chain]
     │
     ├── invalid cert → reject silently, log
     │
     └── valid → show dialog
                   │
                   ├── User approves          → grant issued, data flows
                   ├── User rejects           → request denied, logged
                   └── Timeout                → action per user setting (default allow)
                                                  │
                                                  ├── allow → grant issued (auto_granted=1, audit flagged), data flows
                                                  └── deny  → request denied, logged
                   │
                   ▼
                [Active emergency case opened]
                   │
                   ▼
                [Responder reads patient data, logs interventions]
                   │
                   ▼
                [End of call: Handoff to receiving facility, OR Close case]
                   │
                   ▼
                [Case ends, grant transitions to read-only on case span]
                   │
                   ▼
                [Notification to patient: "Emergency access ended. [Review case]"]
```

---

## Designer's handoff notes

- Aesthetic per `ux-design.md`: black / white / red palette, type-driven hierarchy, Outfit + Inter + JetBrains Mono.
- The emergency dialog is the highest-stakes UI in the entire OHD product. Consider it a first-priority design area.
- The "auto-granted" badge in the audit / case views needs a distinct visual treatment — different color (perhaps amber or muted red), small icon, hover/tap text explaining the timeout-default-allow path. The user must be able to identify these accesses at a glance to review them after the fact.
- The countdown timer in the dialog should be visually prominent without being distressing — emergencies are stressful enough already.
- Lock-screen-basic-only mode is a power-user feature; default to the full above-lock dialog for the typical user.
- The settings copy ("Allow gives access if you can't respond..." vs "Refuse access on timeout...") needs careful wording. The defaults should be the right answer for most users, with the alternative clearly marked as a tradeoff for specific concerns.
- Bystander proxy role (the "Good Samaritan" feature) is on by default with a clear setting to opt out. UX-wise this should be barely noticeable — bystanders shouldn't be disrupted by emergency events for strangers; the feature works silently in the background.
