# OHD Connect Web — Status / Handoff

> Personal-side SPA at `connect/web/`. Vite + React 18 + TypeScript +
> react-router-dom + `@connectrpc/connect-web` against the storage component's
> wire schema (`storage/proto/ohdc/v0/`).

## OHDC wire/API version renamed to v0 (2026-05-09)

The web client and Buf notes now target generated OHDC artifacts under
`src/gen/ohdc/v0/` and the `ohdc.v0` protocol package.

## Date
2026-05-09

## 2026-05-09 — F (pending read queries) and G (light theme) landed

Two of the open items below are now closed:

- **F. `require_approval_per_query` UI** — new `/pending-queries` route +
  `PendingQueriesPage` mirrors the write-approval shape from `PendingPage`.
  Per-row approve / reject (with optional reason); multi-select +
  sticky bulk-action bar at the bottom. AppShell sidebar now distinguishes
  **Pending writes** (`/pending`) from **Pending reads** (`/pending-queries`),
  each with its own badge count. ✅
  - Wire status: storage core exposes `list/approve/reject_pending_query`
    helpers (storage `STATUS.md`, migration `005_pending_queries.sql`),
    but the corresponding `OhdcService.{List,Approve,Reject}PendingQuery`
    RPCs are NOT yet in `storage/proto/ohdc/v0/ohdc.proto`. Connect-web
    falls back to an in-memory mock store keyed off
    `pendingQueriesIsMock()`. The page renders an explicit "mock" banner
    so reviewers can tell. The probe in `client.ts` flips automatically
    when the proto sweep adds the RPCs and `pnpm gen` re-runs.
  - The "trust this query forever" affordance from `PendingPage` is
    deliberately omitted: that would defeat `require_approval_per_query`.
    A v0.x add-on could grow per-pattern auto-approve rules on the
    grant; out of scope here.

- **G. Light theme toggle** — new Settings → Appearance sub-page with a
  three-way picker (System / Dark / Light). Persists to
  `localStorage["ohd-connect-theme"]`. The CSS already had
  `[data-theme="light"]` variables; this lights it up. The "System"
  mode reactively follows the OS `prefers-color-scheme` media query.
  `bootstrapTheme()` runs from `main.tsx` BEFORE React mounts, so
  there's no flash of wrong theme on first paint. ✅

## What's wired

The five primary tabs (Log / Dashboard / Grants / Pending / Settings) all
mount and render a v0 UI against the OHDC client. Self-session auth lands as
either `?token=ohds_…` on the URL (one-shot, stripped after copy) or via the
paste-token field on the Storage settings page. Both flow into
`sessionStorage` and bootstrap re-runs.

| Tab | OHDC RPCs called | Status |
|---|---|---|
| **Log** | `Events.PutEvents` for glucose, HR, BP, temperature, medication, symptom, meal, mood, clinical_note | Wired |
| **Dashboard** | `Events.QueryEvents` (bootstrap) | Wired (sparklines for glucose / HR / temp / BP; recent-50 list) |
| **Grants** | `Grants.CreateGrant`, `Grants.ListGrants`, `Grants.RevokeGrant` | Wired (5 templates, share-sheet, revoke) |
| **Pending writes** | `Pending.ListPending`, `Pending.ApprovePending`, `Pending.RejectPending` | Wired (approve, approve-and-trust-type, reject with reason) |
| **Pending reads** | `Pending.{List,Approve,Reject}PendingQuery` (proto pending) | Wired against in-memory mock; auto-flips to wire path when proto exposes the RPCs |
| **Settings → Storage** | `Diag.Health`, `Diag.WhoAmI` | Wired |
| **Settings → Emergency** | (local-only — see "What's stubbed") | Form persists to localStorage |
| **Settings → Cases** | `Cases.ListCases`, `Cases.CloseCase` | Wired (force-close); retro-grants TBD |
| **Settings → Delegates** | `Grants.CreateGrant` (`granteeKind=delegate`), `Grants.RevokeGrant` | Wired as v0 stand-in; swap to dedicated `IssueDelegateGrant` proto extension when storage ships it |
| **Settings → Export** | (none) | TBD until storage Export ships |
| **Settings → Appearance** | (none) | Wired — three-way theme toggle persisted to `localStorage` |

That's **14** of the 21 storage-wired RPCs the user mentioned. The rest
(`AttachBlob`, `ReadAttachment`, `ReadSamples`, `Aggregate`, `Correlate`,
`AuditQuery`, `Export`, `Import`, full Cases CRUD beyond ListCases/CloseCase)
either return `Unimplemented` from the storage server (per
`storage/STATUS.md`) or aren't needed for the v0.1 surface. The UI surfaces
them as "TBD" affordances where relevant (per-grant audit, export buttons,
issue retrospective grant).

## UX choices made

### Visual style
- **Dark theme by default** per `ux-design.md` palette (`#E11D2A` red
  accent, `#0A0A0A` ink, `#F5F5F5` text on dark). A
  `[data-theme="light"]` override is in `src/index.css` ready for a
  v0.x toggle but not exposed in UI yet.
- System font stack (no Outfit / Inter / JetBrains Mono pulled in — keeps
  the bundle small and avoids a layout-shift on first paint). Type-driven
  hierarchy: thin-weight headings, mono for ULIDs/values.
- No emojis in copy or icons; minimal geometric Unicode glyphs (`◐ ♡ ◇`)
  as tile / nav icons. Final icon pass is open for a designer.

### Layout
- **Mobile-first** single-column. Bottom-bar navigation with 5 items
  (Log / Dashboard / Grants / Pending / Settings) on phones; sidebar takes
  over on desktop (>= 880px).
- Top bar is sticky, shows brand, truncated user-ULID, connectivity dot,
  and a Sign-out button when a token is present.
- Modals are bottom-sheets on mobile (rounded-top, full-width) and
  centered dialogs on desktop. Forms use type-appropriate inputs (number,
  range, select).

### Storyboards
- **Log**: 9-tile grid (4×2 on phones once it has space, 4×3 on desktop).
  Tap → typed modal → Submit → toast confirms. Glucose / temp let you
  pick the unit, conversion happens client-side before the OHDC call.
  Symptom + Mood use a 1–10 range slider (live readout in the label).
- **Dashboard**: Top header line ("N events loaded — last write Xm ago"),
  then a card per measurement event-type with a 60-px-tall SVG
  sparkline + the current value, then a card with the most-recent 50
  events as a list (`fmtRelative` time, type label, channel summary).
- **Grants**: Page header with `+ New grant`. Card per active grant
  showing label, granteeKind, approvalMode, expiry status (active /
  expiring soon / expired), full read+write scope, last-used. Actions:
  View audit (TBD), Revoke (with confirm dialog). New-grant modal:
  template picker → label input → defaults preview (kv-grid) → Issue.
  Post-issue share-sheet: warn-banner ("only shown once"), copy-paste
  fields for token + share-URL.
- **Pending**: Empty when quiet. Otherwise a card per pending event
  showing submitter (grant ULID), submitted-relative time, event type,
  and a structured table of channels. Three buttons: Approve, Approve &
  trust type, Reject (expanded inline with optional reason textarea).
- **Settings → Emergency**: Eight `<Section>` cards mapping 1:1 to the
  designer-handoff screens-emergency.md sections. Feature toggle gates
  the rest (everything below is greyed at 85% opacity when off). Includes
  the approval timeout slider (10–300s, current value in the title), the
  Allow-vs-Refuse default dropdown, lock-screen mode dropdown, history
  window (0/3/12/24h), per-channel toggles for the standard emergency
  profile (allergies, meds, blood type, advance directives, diagnoses,
  glucose, HR, BP, SpO₂, temperature), sensitivity-class toggles
  (general ON, mental_health/substance_use/sexual_health/reproductive
  OFF), location share opt-in, trust-roots list (built-in OHD root +
  user-added), bystander-proxy toggle, reset-to-defaults, big "Disable
  emergency feature" button. **All persists to `localStorage` for v0.1**;
  the wire-up to `Settings.SetEmergencyConfig` lands once storage ships
  that RPC.

### Auth UX
- One-shot URL token: open
  `http://localhost:5174/?token=ohds_…&storage=http://localhost:8443`
  to populate sessionStorage, then the URL is rewritten to remove the
  query params (so reload + bookmark don't leak).
- Paste-token form on the Storage settings page; survives reload until
  tab close.
- No-token state: `/no-token` route renders explanatory copy + how to
  mint one with `ohd-storage-server issue-self-token`. The bootstrap
  gate routes here whenever `resolveSelfToken()` returns null.
- Sign-out clears sessionStorage, bumps the snapshot, redirects to `/`.

## What's stubbed / deferred

| Area | Why | Pickup |
|---|---|---|
| **Real OAuth flow** (Authorization Code + PKCE per `connect/spec/auth.md`) | Storage `/authorize` / `/token` / `/oauth/register` not yet shipped (storage `STATUS.md` "HTTP-only OAuth/discovery endpoints: v1.x"). | Pin `oauth4webapi`. Wire a `/login` route that opens the AS, lands on `/oidc-callback`, exchanges the code, persists `(ohds_, ohdr_)` in IndexedDB origin-isolated. |
| **Per-grant audit view** (Grants tab → "View audit" button) | `Audit.AuditQuery` returns `Unimplemented`. | Drop in a streaming-list view; the page already has the affordance. |
| **Sample charts** (denser than sparklines for HR/glucose density) | `Events.ReadSamples` is stubbed. | Either swap the sparkline for a richer line chart against decoded samples, or keep using `QueryEvents` aggregated into per-channel arrays. |
| **`Aggregate` / `Correlate`** (e.g. meal/glucose-response view) | Storage RPCs stubbed. | Add a "/insights" tab (or a section on Dashboard). |
| **`AttachBlob` / `ReadAttachment`** (photo for symptom logs, ECG bytes) | Storage RPCs stubbed. | LogPage's symptom modal grows an attachment picker. |
| **`Export` / `Import`** | Storage RPCs stubbed. | Settings → Export page is the landing point; just remove the disabled state and wire the streaming download. |
| **Cases CRUD beyond `ListCases` + `CloseCase`** (CreateCase works for the open-case button, ReopenCase + AddCaseFilter + GetCase) | Storage-side incomplete (`storage/STATUS.md` "Cases CRUD: v1.x"). | Cases settings page already has TBD buttons for retrospective grants and case detail views. |
| **`Settings.SetEmergencyConfig`** (the storage RPC the emergency form should write through) | Not in the proto today; the per-user emergency-template grant exists but the management RPC is v0.x. | EmergencySettingsPage's `setS()` write-through path replaces `localStorage` with a CreateGrant/UpdateGrant call against the user's emergency template. |
| **Web Push notifications** (per `connect/spec/notifications.md`) | Service worker, VAPID keys, push subscription. | Add `public/sw.js` + a `pushRegistration.ts` module that calls a future `Notify.RegisterDevice` RPC. |
| **Barcode scanner** (food log) | Browser `BarcodeDetector` API — not in the v0.1 LogPage. | Add a barcode tile that opens the camera and resolves via OpenFoodFacts (`spec/openfoodfacts.md`). |
| ~~**Light theme toggle**~~ | ✅ **Landed (2026-05-09).** Settings → Appearance: three-way picker (System / Dark / Light) persisted to `localStorage["ohd-connect-theme"]`; bootstrap-time apply in `main.tsx` so no flash of wrong theme. |
| **Bulk approve / reject** in PendingPage | One-by-one in v0.1. | Add a checkbox column + a sticky bottom "Approve N" / "Reject N" bar. The new PendingQueriesPage already ships this shape — port back to PendingPage. |
| ~~**`require_approval_per_query` UI**~~ | ✅ **Landed (2026-05-09)** as `/pending-queries` (PendingQueriesPage). Currently against an in-memory mock — storage core has the helpers, the proto RPCs land in v1.x; client probes for the wire surface and swaps automatically. |

## Verify

```
cd connect/web
pnpm install         # ~1-2 min first time
pnpm gen             # codegens TS Connect-Web client into src/gen/ (~5s)
pnpm typecheck       # clean
pnpm test            # vitest run — currently 21 tests pass
pnpm build           # production bundle to dist/ (~404 KB / 121 KB gzipped)
pnpm dev             # http://localhost:5174 (avoids care/web's :5173)
```

## Tree (3 levels)

```
connect/web/
├── README.md
├── STATUS.md
├── buf.gen.yaml
├── index.html
├── package.json
├── tsconfig.json
├── vite.config.ts
└── src/
    ├── App.tsx
    ├── index.css
    ├── main.tsx
    ├── theme.ts                 ← NEW (G)
    ├── util.ts
    ├── components/
    │   ├── AppShell.tsx
    │   ├── Modal.tsx
    │   ├── Sparkline.tsx
    │   └── Toast.tsx
    ├── ohdc/
    │   ├── client.ts            ← extended with PendingQuery + mock fallback
    │   ├── oidc.ts
    │   ├── store.ts             ← snapshot.pendingQueries, bulk* helpers
    │   └── useStore.ts
    ├── pages/
    │   ├── DashboardPage.tsx
    │   ├── GrantsPage.tsx
    │   ├── LogPage.tsx
    │   ├── LoginPage.tsx
    │   ├── OidcCallbackPage.tsx
    │   ├── PendingPage.tsx
    │   ├── PendingQueriesPage.tsx ← NEW (F)
    │   └── settings/
    │       ├── AppearanceSettingsPage.tsx ← NEW (G)
    │       ├── CasesSettingsPage.tsx
    │       ├── DelegatesSettingsPage.tsx
    │       ├── EmergencySettingsPage.tsx
    │       ├── ExportSettingsPage.tsx
    │       ├── SettingsLayout.tsx
    │       └── StorageSettingsPage.tsx
    └── test/
        ├── pages.test.tsx       ← +5 tests for F + G
        ├── setup.ts
        ├── smoke.test.tsx
        └── store.test.ts
```

## Decisions to flag

1. **Dark default**. The Connect Android app defaults to Material3 dark
   per `connect/android/.../MainActivity.kt`; we mirror that on web.
2. **Bottom-bar nav on mobile, sidebar on desktop**, as described in the
   user's design brief. Five items keep each tab reachable in one tap.
3. **Sparklines, not a chart lib**. Matches care/web's
   no-dependency-bloat aesthetic; the bundle stays under 250 KB gzipped.
4. **No `react-router-dom` `useMatch` wrappers in pages** — every page
   reads the snapshot directly via `useStoreVersion()`, so no shared
   active-context state was needed (unlike care/web's active-patient
   pattern).
5. **`localStorage` for emergency settings**. The store's
   `Settings.SetEmergencyConfig` RPC isn't in the proto yet; v0.1 keeps
   the form data local until then. STATUS.md flags this as the
   integration point.
6. **Token in `sessionStorage`** (closes-with-tab) rather than
   `localStorage` (persists). Per `connect/spec/auth.md` the
   browser-side requirement is "IndexedDB with origin isolation, refresh
   token never exposed to JS"; v0.1's `sessionStorage` is a placeholder
   for the IndexedDB-backed module that lands with the OAuth code flow.
   Trade-off: reload survives, tab close logs you out — pragmatic for
   v0.1.
7. **One snapshot, one bootstrap promise**. Care/web's pattern. No
   per-tab data fetching; everything is on the bootstrap path. Refresh
   on writes; explicit `reBootstrap()` after token / URL rotation.
8. **Five grant templates** baked into `store.ts` per
   `connect/SPEC.md` "Grant management UX → Templates". Each template's
   defaults are overridable on the create-grant form (overrides parameter
   on `createGrantFromTemplate`). The default-action / approval-mode are
   defended ("revoke + re-create instead" per the proto comment), the
   rest is sparse-overridable.

## Recommended next steps

1. Wire real OAuth (storage HTTP surface lands first; one-week pickup
   here once it does).
2. Add `AuditQuery` consumption when storage wires it — Grants page
   already has the button.
3. Replace the per-channel sparklines with `Aggregate`-driven mini-charts
   (one storage RPC away).
4. Add Web Push registration for Pending events (notify-on-pending).
5. Add the chart-library equivalent of `ReadSamples` for dense series
   (HR-during-exercise) — even a simple uPlot drop-in works.
6. Wire `Settings.SetEmergencyConfig` once it exists — the form is
   already shaped right.
