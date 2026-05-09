// Personal-side store, backed by OHDC under self-session auth.
//
// The store is a tiny event-emitter the React layer subscribes to via
// `useSyncExternalStore`. Snapshot is hydrated by `bootstrap()` (Health +
// WhoAmI + initial event read) and refreshed on every write (optimistic
// insert + reconcile from the server).
//
// Surface mirrors the spec's "Connect personal app" — log, dashboard,
// grants, pending review, cases. Settings are routed directly through the
// client (no caching needed).

import {
  approvePending,
  approvePendingQuery,
  closeCase,
  createCase,
  createGrant,
  forgetSelfToken,
  healthCheck,
  listCases,
  listGrants,
  listPending,
  listPendingQueries,
  putEvent,
  queryEvents,
  rejectPending,
  rejectPendingQuery,
  resolveSelfToken,
  revokeGrant,
  ulidToCrockford,
  whoAmI,
  type Case,
  type CreateGrantOpts,
  type Event,
  type Grant,
  type PendingEvent,
  type PendingQuery,
  type WhoAmIResponse,
} from "./client";

// --- Event types we read for the dashboard ---------------------------------

/**
 * The std.* event types we read for the dashboard / log timeline. Matches
 * the registry seed in `storage/migrations/002_std_registry.sql` plus the
 * `std.clinical_note` row added by the storage server's
 * `issue-grant-token` op-shortcut.
 */
export const READ_EVENT_TYPES = [
  "std.blood_glucose",
  "std.heart_rate_resting",
  "std.body_temperature",
  "std.blood_pressure",
  "std.medication_dose",
  "std.symptom",
  "std.meal",
  "std.mood",
  "std.clinical_note",
];

// --- Snapshot --------------------------------------------------------------

interface Snapshot {
  ready: boolean;
  /** "no_token" | "whoami_failed" | "bootstrap_failed: <msg>" | null */
  error: string | null;
  me: WhoAmIResponse | null;
  events: Event[];
  grants: Grant[];
  pending: PendingEvent[];
  /** Pending **read** queries (require_approval_per_query). */
  pendingQueries: PendingQuery[];
  cases: Case[];
  /** Connectivity / version readout from `Health`. */
  health: {
    status: string;
    serverVersion: string;
    protocolVersion: string;
    serverTimeMs: number;
  } | null;
}

let snapshot: Snapshot = {
  ready: false,
  error: null,
  me: null,
  events: [],
  grants: [],
  pending: [],
  pendingQueries: [],
  cases: [],
  health: null,
};

const subscribers = new Set<() => void>();
let version = 0;

function notify() {
  version += 1;
  for (const s of subscribers) s();
}

export function subscribe(fn: () => void): () => void {
  subscribers.add(fn);
  return () => {
    subscribers.delete(fn);
  };
}

export function getVersion(): number {
  return version;
}

export function getSnapshot(): Snapshot {
  return snapshot;
}

export function getBootstrapStatus(): { ready: boolean; error: string | null } {
  return { ready: snapshot.ready, error: snapshot.error };
}

// --- Bootstrap -------------------------------------------------------------

let bootstrapping: Promise<void> | null = null;

/**
 * Runs Health + WhoAmI + initial QueryEvents/Grants/Pending/Cases. Idempotent;
 * later calls return the cached promise.
 */
export function bootstrap(): Promise<void> {
  if (bootstrapping) return bootstrapping;
  bootstrapping = (async () => {
    const token = resolveSelfToken();
    if (!token) {
      snapshot = { ...snapshot, ready: true, error: "no_token" };
      notify();
      return;
    }
    try {
      const [health, me] = await Promise.all([healthCheck(), whoAmI()]);
      if (!me) {
        snapshot = {
          ...snapshot,
          ready: true,
          error: "whoami_failed",
          health: health
            ? {
                status: health.status,
                serverVersion: health.serverVersion,
                protocolVersion: health.protocolVersion,
                serverTimeMs: Number(health.serverTimeMs),
              }
            : null,
        };
        notify();
        return;
      }
      const [events, grants, pending, pendingQueries, cases] = await Promise.all([
        queryEvents({ eventTypes: READ_EVENT_TYPES, limit: 1000 }).catch((e) => {
          // eslint-disable-next-line no-console
          console.warn("queryEvents bootstrap failed", e);
          return [] as Event[];
        }),
        listGrants(false).catch((e) => {
          // eslint-disable-next-line no-console
          console.warn("listGrants bootstrap failed", e);
          return [] as Grant[];
        }),
        listPending("pending").catch((e) => {
          // eslint-disable-next-line no-console
          console.warn("listPending bootstrap failed", e);
          return [] as PendingEvent[];
        }),
        listPendingQueries().catch((e) => {
          // eslint-disable-next-line no-console
          console.warn("listPendingQueries bootstrap failed", e);
          return [] as PendingQuery[];
        }),
        listCases(true).catch((e) => {
          // eslint-disable-next-line no-console
          console.warn("listCases bootstrap failed", e);
          return [] as Case[];
        }),
      ]);
      snapshot = {
        ready: true,
        error: null,
        me,
        events: sortEventsDesc(events),
        grants,
        pending,
        pendingQueries,
        cases,
        health: health
          ? {
              status: health.status,
              serverVersion: health.serverVersion,
              protocolVersion: health.protocolVersion,
              serverTimeMs: Number(health.serverTimeMs),
            }
          : null,
      };
      notify();
    } catch (err) {
      // eslint-disable-next-line no-console
      console.error("OHDC bootstrap failed", err);
      snapshot = {
        ...snapshot,
        ready: true,
        error: `bootstrap_failed: ${(err as Error).message ?? String(err)}`,
      };
      notify();
    }
  })();
  return bootstrapping;
}

/** Re-run the read pass after a write. Best-effort; logs on failure. */
export async function refresh(): Promise<void> {
  const token = resolveSelfToken();
  if (!token) return;
  try {
    const [events, grants, pending, pendingQueries, cases] = await Promise.all([
      queryEvents({ eventTypes: READ_EVENT_TYPES, limit: 1000 }).catch(() => snapshot.events),
      listGrants(false).catch(() => snapshot.grants),
      listPending("pending").catch(() => snapshot.pending),
      listPendingQueries().catch(() => snapshot.pendingQueries),
      listCases(true).catch(() => snapshot.cases),
    ]);
    snapshot = {
      ...snapshot,
      events: sortEventsDesc(events),
      grants,
      pending,
      pendingQueries,
      cases,
    };
    notify();
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error("OHDC refresh failed", err);
  }
}

/** Restart bootstrap from scratch — used after token / URL rotation. */
export function reBootstrap(): Promise<void> {
  bootstrapping = null;
  snapshot = {
    ready: false,
    error: null,
    me: null,
    events: [],
    grants: [],
    pending: [],
    pendingQueries: [],
    cases: [],
    health: null,
  };
  notify();
  return bootstrap();
}

/** Sign-out helper: forget the token and reset the snapshot. */
export function signOut(): void {
  forgetSelfToken();
  bootstrapping = null;
  snapshot = {
    ready: true,
    error: "no_token",
    me: null,
    events: [],
    grants: [],
    pending: [],
    pendingQueries: [],
    cases: [],
    health: null,
  };
  notify();
}

// --- Selectors -------------------------------------------------------------

export function getRecentEvents(limit = 50): Event[] {
  return snapshot.events.slice(0, limit);
}

export function getEventsByType(eventType: string, limit = 50): Event[] {
  return snapshot.events.filter((e) => e.eventType === eventType).slice(0, limit);
}

export function getMe(): WhoAmIResponse | null {
  return snapshot.me;
}

export function getMyUserUlid(): string {
  const ulid = snapshot.me?.userUlid?.bytes;
  return ulid ? ulidToCrockford(ulid) : "";
}

// --- Submit helpers --------------------------------------------------------

/**
 * Generic submit + optimistic refresh. The write fires through OHDC; once
 * we get a result back (committed / pending / error) we refetch. Optimistic
 * UI is achieved by re-rendering `events` from the cached snapshot — for
 * v0.1 we simply refresh to get the canonical state after each write.
 */
export async function submitGlucose(valueMmolL: number, notes?: string): Promise<void> {
  await putEvent({
    eventType: "std.blood_glucose",
    timestampMs: Date.now(),
    channels: [{ channelPath: "value", value: { kind: "real", realValue: valueMmolL } }],
    notes,
  });
  void refresh();
}

export async function submitGlucoseMgDl(valueMgDl: number, notes?: string): Promise<void> {
  // mg/dL → mmol/L (canonical) per the storage CLI: `mg/dL ÷ 18.0182`.
  const mmol = valueMgDl / 18.0182;
  return submitGlucose(mmol, notes);
}

export async function submitHeartRate(bpm: number, notes?: string): Promise<void> {
  await putEvent({
    eventType: "std.heart_rate_resting",
    timestampMs: Date.now(),
    channels: [{ channelPath: "bpm", value: { kind: "real", realValue: bpm } }],
    notes,
  });
  void refresh();
}

export async function submitTemperatureC(celsius: number, notes?: string): Promise<void> {
  await putEvent({
    eventType: "std.body_temperature",
    timestampMs: Date.now(),
    channels: [{ channelPath: "value", value: { kind: "real", realValue: celsius } }],
    notes,
  });
  void refresh();
}

export async function submitTemperatureF(fahrenheit: number, notes?: string): Promise<void> {
  return submitTemperatureC(((fahrenheit - 32) * 5) / 9, notes);
}

export async function submitBloodPressure(systolic: number, diastolic: number, notes?: string): Promise<void> {
  await putEvent({
    eventType: "std.blood_pressure",
    timestampMs: Date.now(),
    channels: [
      { channelPath: "systolic", value: { kind: "real", realValue: systolic } },
      { channelPath: "diastolic", value: { kind: "real", realValue: diastolic } },
    ],
    notes,
  });
  void refresh();
}

export async function submitMedication(name: string, doseMg: number | null, status: "taken" | "skipped" | "late" | "refused" = "taken"): Promise<void> {
  const channels: Parameters<typeof putEvent>[0]["channels"] = [
    { channelPath: "name", value: { kind: "text", textValue: name } },
    { channelPath: "status", value: { kind: "text", textValue: status } },
  ];
  if (doseMg != null) {
    channels.push({ channelPath: "dose", value: { kind: "real", realValue: doseMg } });
    channels.push({ channelPath: "dose_unit", value: { kind: "text", textValue: "mg" } });
  }
  await putEvent({
    eventType: "std.medication_dose",
    timestampMs: Date.now(),
    channels,
  });
  void refresh();
}

export async function submitSymptom(name: string, severity: number, notes?: string): Promise<void> {
  await putEvent({
    eventType: "std.symptom",
    timestampMs: Date.now(),
    channels: [
      { channelPath: "name", value: { kind: "text", textValue: name } },
      { channelPath: "severity", value: { kind: "int", intValue: severity } },
    ],
    notes,
  });
  void refresh();
}

export async function submitMood(mood: string, energy: number | null, notes?: string): Promise<void> {
  const channels: Parameters<typeof putEvent>[0]["channels"] = [
    { channelPath: "mood", value: { kind: "text", textValue: mood } },
  ];
  if (energy != null) channels.push({ channelPath: "energy", value: { kind: "int", intValue: energy } });
  await putEvent({
    eventType: "std.mood",
    timestampMs: Date.now(),
    channels,
    notes,
  });
  void refresh();
}

export async function submitMeal(description: string, kcal: number | null, notes?: string): Promise<void> {
  const channels: Parameters<typeof putEvent>[0]["channels"] = [
    { channelPath: "description", value: { kind: "text", textValue: description } },
  ];
  if (kcal != null) channels.push({ channelPath: "kcal", value: { kind: "real", realValue: kcal } });
  await putEvent({
    eventType: "std.meal",
    timestampMs: Date.now(),
    channels,
    notes,
  });
  void refresh();
}

export async function submitNote(text: string): Promise<void> {
  await putEvent({
    eventType: "std.clinical_note",
    timestampMs: Date.now(),
    channels: [
      { channelPath: "text", value: { kind: "text", textValue: text } },
      { channelPath: "author", value: { kind: "text", textValue: "self" } },
    ],
    notes: text,
  });
  void refresh();
}

// --- Grant management ------------------------------------------------------

/** Grant-creation templates surfaced by the Grants tab. Resolved client-side. */
export type GrantTemplateId =
  | "primary_doctor"
  | "specialist_visit"
  | "spouse_family"
  | "researcher"
  | "emergency_break_glass";

export interface GrantTemplateDefaults {
  granteeKind: string;
  approvalMode: CreateGrantOpts["approvalMode"];
  defaultAction: CreateGrantOpts["defaultAction"];
  expiresInDays: number | null;
  readEventTypes: string[];
  writeEventTypes: string[];
  autoApproveEventTypes: string[];
  denySensitivityClasses: string[];
  notifyOnAccess: boolean;
  stripNotes: boolean;
  aggregationOnly: boolean;
}

const ALL_READ: string[] = READ_EVENT_TYPES;
const SENSITIVE_CLASSES_OPTIONAL = [
  "mental_health",
  "substance_use",
  "sexual_health",
  "reproductive",
];

/**
 * Template defaults per `connect/SPEC.md` "Grant management UX → Templates".
 * The user can override any field on the create-grant form.
 */
export const GRANT_TEMPLATES: Record<GrantTemplateId, GrantTemplateDefaults & { label: string; sub: string }> = {
  primary_doctor: {
    label: "Primary doctor",
    sub: "All channels, 1-year expiry, auto-approve labs/notes.",
    granteeKind: "user",
    approvalMode: "auto_for_event_types",
    defaultAction: "allow",
    expiresInDays: 365,
    readEventTypes: ALL_READ,
    writeEventTypes: ["std.clinical_note", "std.medication_dose"],
    autoApproveEventTypes: ["std.clinical_note"],
    denySensitivityClasses: SENSITIVE_CLASSES_OPTIONAL,
    notifyOnAccess: false,
    stripNotes: false,
    aggregationOnly: false,
  },
  specialist_visit: {
    label: "Specialist for one visit",
    sub: "30-day scope, every write needs your approval.",
    granteeKind: "user",
    approvalMode: "always",
    defaultAction: "allow",
    expiresInDays: 30,
    readEventTypes: ALL_READ,
    writeEventTypes: ["std.clinical_note"],
    autoApproveEventTypes: [],
    denySensitivityClasses: SENSITIVE_CLASSES_OPTIONAL,
    notifyOnAccess: false,
    stripNotes: false,
    aggregationOnly: false,
  },
  spouse_family: {
    label: "Spouse / family",
    sub: "Read-only, vitals + emergency profile, indefinite.",
    granteeKind: "user",
    approvalMode: "always",
    defaultAction: "allow",
    expiresInDays: null,
    readEventTypes: ["std.heart_rate_resting", "std.body_temperature", "std.blood_pressure", "std.blood_glucose"],
    writeEventTypes: [],
    autoApproveEventTypes: [],
    denySensitivityClasses: [...SENSITIVE_CLASSES_OPTIONAL],
    notifyOnAccess: true,
    stripNotes: false,
    aggregationOnly: false,
  },
  researcher: {
    label: "Researcher with study",
    sub: "Aggregation only, strip notes, study window.",
    granteeKind: "researcher",
    approvalMode: "always",
    defaultAction: "allow",
    expiresInDays: 90,
    readEventTypes: ALL_READ,
    writeEventTypes: [],
    autoApproveEventTypes: [],
    denySensitivityClasses: SENSITIVE_CLASSES_OPTIONAL,
    notifyOnAccess: true,
    stripNotes: true,
    aggregationOnly: true,
  },
  emergency_break_glass: {
    label: "Emergency break-glass",
    sub: "Template for first responders. Ships as a template grant; cloned on incident.",
    granteeKind: "emergency_authority",
    approvalMode: "auto_for_event_types",
    defaultAction: "allow",
    expiresInDays: 1,
    readEventTypes: [
      "std.blood_glucose",
      "std.heart_rate_resting",
      "std.body_temperature",
      "std.blood_pressure",
      "std.medication_dose",
    ],
    writeEventTypes: [],
    autoApproveEventTypes: [],
    denySensitivityClasses: SENSITIVE_CLASSES_OPTIONAL,
    notifyOnAccess: true,
    stripNotes: false,
    aggregationOnly: false,
  },
};

export async function createGrantFromTemplate(
  templateId: GrantTemplateId,
  granteeLabel: string,
  overrides: Partial<CreateGrantOpts> = {},
): Promise<{ token: string; shareUrl: string; grantUlid: string }> {
  const t = GRANT_TEMPLATES[templateId];
  const opts: CreateGrantOpts = {
    granteeLabel,
    granteeKind: t.granteeKind,
    purpose: overrides.purpose,
    defaultAction: t.defaultAction,
    approvalMode: t.approvalMode,
    expiresAtMs:
      overrides.expiresAtMs ??
      (t.expiresInDays != null ? Date.now() + t.expiresInDays * 86_400_000 : undefined),
    notifyOnAccess: t.notifyOnAccess,
    stripNotes: t.stripNotes,
    aggregationOnly: t.aggregationOnly,
    readEventTypes: overrides.readEventTypes ?? t.readEventTypes,
    writeEventTypes: overrides.writeEventTypes ?? t.writeEventTypes,
    autoApproveEventTypes: overrides.autoApproveEventTypes ?? t.autoApproveEventTypes,
    denySensitivityClasses: overrides.denySensitivityClasses ?? t.denySensitivityClasses,
  };
  const result = await createGrant(opts);
  void refresh();
  return { token: result.token, shareUrl: result.shareUrl, grantUlid: result.grantUlid };
}

export async function revokeGrantById(grantUlid: string, reason?: string): Promise<void> {
  await revokeGrant(grantUlid, reason);
  void refresh();
}

// --- Pending review --------------------------------------------------------

export async function approvePendingById(pendingUlid: string, alsoTrustType = false): Promise<void> {
  await approvePending(pendingUlid, alsoTrustType);
  void refresh();
}

export async function rejectPendingById(pendingUlid: string, reason?: string): Promise<void> {
  await rejectPending(pendingUlid, reason);
  void refresh();
}

// --- Pending read queries (require_approval_per_query) ---------------------
//
// Mirror of the write-approval helpers above. Calls into client.ts which
// auto-falls-back to an in-memory mock until the proto exposes the wire RPCs.

export async function approvePendingQueryById(queryUlid: string): Promise<void> {
  await approvePendingQuery(queryUlid);
  void refresh();
}

export async function rejectPendingQueryById(queryUlid: string, reason?: string): Promise<void> {
  await rejectPendingQuery(queryUlid, reason);
  void refresh();
}

/**
 * Bulk approve a list of pending-query ULIDs. Returns the count that
 * succeeded; failures are toast-reported by the caller. We run these
 * sequentially rather than `Promise.all` to keep the audit log ordered.
 */
export async function bulkApprovePendingQueries(queryUlids: string[]): Promise<{ ok: number; failed: number }> {
  let ok = 0;
  let failed = 0;
  for (const u of queryUlids) {
    try {
      await approvePendingQuery(u);
      ok++;
    } catch (err) {
      // eslint-disable-next-line no-console
      console.warn("bulk approve failed for", u, err);
      failed++;
    }
  }
  void refresh();
  return { ok, failed };
}

export async function bulkRejectPendingQueries(
  queryUlids: string[],
  reason?: string,
): Promise<{ ok: number; failed: number }> {
  let ok = 0;
  let failed = 0;
  for (const u of queryUlids) {
    try {
      await rejectPendingQuery(u, reason);
      ok++;
    } catch (err) {
      // eslint-disable-next-line no-console
      console.warn("bulk reject failed for", u, err);
      failed++;
    }
  }
  void refresh();
  return { ok, failed };
}

// --- Cases -----------------------------------------------------------------

export async function closeCaseById(caseUlid: string, reason?: string): Promise<void> {
  await closeCase(caseUlid, reason);
  void refresh();
}

export async function openCase(caseType: string, label?: string): Promise<void> {
  await createCase(caseType, label);
  void refresh();
}

// --- Helpers ---------------------------------------------------------------

function sortEventsDesc(events: Event[]): Event[] {
  return [...events].sort((a, b) => Number(b.timestampMs) - Number(a.timestampMs));
}

// --- Test hooks ------------------------------------------------------------

/** Replace the snapshot directly. Tests only. */
export function _setSnapshotForTesting(s: Partial<Snapshot>): void {
  snapshot = { ...snapshot, ...s };
  notify();
}

/** Reset all in-memory state. Tests only. */
export function _resetForTesting(): void {
  bootstrapping = null;
  snapshot = {
    ready: false,
    error: null,
    me: null,
    events: [],
    grants: [],
    pending: [],
    pendingQueries: [],
    cases: [],
    health: null,
  };
  version = 0;
  subscribers.clear();
}
