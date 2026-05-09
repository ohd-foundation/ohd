import { useState } from "react";
import { Modal } from "../../components/Modal";
import { useToast } from "../../components/Toast";
import { fmtDate, fmtDays } from "../../util";
import { useStoreVersion } from "../../ohdc/useStore";
import { getSnapshot, refresh as refreshStore, revokeGrantById } from "../../ohdc/store";
import { createGrant, ulidToCrockford } from "../../ohdc/client";

/**
 * Settings → Delegates.
 *
 * Family / delegate access (per the canonical spec's "open design item":
 * one user acting on behalf of another). A delegate grant lets someone
 * else use *your* OHDC surface to read events / submit writes — useful
 * for caregivers managing an elderly parent, parents managing a child's
 * health record, or a spouse during incapacity.
 *
 * v0 limitations:
 *  - Storage's `IssueDelegateGrant` proto extension is not yet shipped
 *    (`storage/STATUS.md`: "core fn ready, needs the proto field added
 *    to `CreateGrantRequest` (`delegate_for_user_ulid`)"). For now we
 *    issue a regular grant with `granteeKind="delegate"`. Once the proto
 *    add lands, the create-delegate path swaps to `client.issueDelegateGrant`.
 *  - The delegate's OIDC identity is captured as a free-text "paste-token"
 *    pre-shared blob — the delegate-to-be shares their public identity.
 *    Full UX (delegate accepts via QR / NFC tap, OIDC trust check) is
 *    v0.x.
 *
 * Distinct visual treatment per the brief: a yellow "delegate" pill +
 * a separate sub-page so delegates don't blur into the regular Grants
 * list (where they live alongside doctors, researchers, etc.).
 */
export function DelegatesSettingsPage() {
  useStoreVersion();
  const snap = getSnapshot();
  const [creating, setCreating] = useState(false);
  const toast = useToast();

  const delegates = snap.grants.filter((g) => g.granteeKind === "delegate");

  return (
    <div data-testid="settings-delegates">
      <div className="banner info">
        Delegate access lets someone use OHD <em>on your behalf</em> — caregivers, parents
        of minors, or a spouse during incapacity. Each delegate access flows through your
        own OHDC surface, scoped, revocable, fully audit-logged. Storage's dedicated
        <code> IssueDelegateGrant </code>proto extension lands in v0.x; today the page
        issues a regular grant with <code>kind=delegate</code> as the v0 stand-in.
      </div>

      <header className="page-header" style={{ alignItems: "center" }}>
        <div>
          <h2 style={{ margin: 0 }}>Active delegates ({delegates.length})</h2>
        </div>
        <button
          className="btn btn-accent"
          onClick={() => setCreating(true)}
          data-testid="issue-delegate"
        >
          + Issue delegate access
        </button>
      </header>

      {delegates.length === 0 ? (
        <div className="empty">
          <p>
            No delegates configured. Tap <strong>Issue delegate access</strong> to grant
            someone permission to act on your behalf — e.g. "Mom's caregiver".
          </p>
        </div>
      ) : (
        delegates.map((g) => {
          const ulid = ulidToCrockford(g.ulid?.bytes);
          const expiresAt = g.expiresAtMs ? Number(g.expiresAtMs) : null;
          const expired = expiresAt != null && expiresAt < Date.now();
          return (
            <div
              key={ulid}
              className="card"
              data-testid={`delegate-card-${ulid}`}
              style={{ borderLeft: "3px solid var(--color-warn)" }}
            >
              <div className="card-title">
                <div>
                  <h3>
                    {g.granteeLabel}{" "}
                    <span
                      className="badge badge-warn"
                      style={{ marginLeft: 8 }}
                      data-testid="delegate-badge"
                    >
                      delegate
                    </span>
                  </h3>
                  <div className="muted" style={{ fontSize: 12, marginTop: 2 }}>
                    <span className="mono">{ulid.slice(0, 8)}…</span> · approval{" "}
                    <strong>{g.approvalMode}</strong>
                  </div>
                </div>
                <div style={{ display: "flex", gap: 8 }}>
                  {expired ? (
                    <span className="flag flag-warn">expired</span>
                  ) : (
                    <span className="flag flag-active">active</span>
                  )}
                </div>
              </div>

              <dl className="kv-grid">
                <dt>Created</dt>
                <dd>{fmtDate(Number(g.createdAtMs))}</dd>
                <dt>Expires</dt>
                <dd>{expiresAt ? fmtDate(expiresAt) : "indefinite"}</dd>
                <dt>Read scope</dt>
                <dd>
                  {g.eventTypeRules
                    .filter((r) => r.effect === "allow")
                    .map((r) => r.eventType)
                    .join(", ") || "(default)"}
                </dd>
                <dt>Write scope</dt>
                <dd>
                  {g.writeEventTypeRules
                    .filter((r) => r.effect === "allow")
                    .map((r) => r.eventType)
                    .join(", ") || "(none)"}
                </dd>
                <dt>Sensitivity-class deny</dt>
                <dd>
                  {g.sensitivityRules
                    .filter((r) => r.effect === "deny")
                    .map((r) => r.sensitivityClass)
                    .join(", ") || "(none)"}
                </dd>
                <dt>Last used</dt>
                <dd>{g.lastUsedMs ? fmtDate(Number(g.lastUsedMs)) : "never"}</dd>
              </dl>

              <div style={{ display: "flex", gap: 8, marginTop: 12, flexWrap: "wrap" }}>
                {!expired ? (
                  <button
                    className="btn btn-sm btn-danger"
                    type="button"
                    onClick={async () => {
                      if (!confirm(`Revoke delegate access for ${g.granteeLabel}?`)) return;
                      try {
                        await revokeGrantById(ulid, "user_revoked_delegate");
                        toast.show("Delegate access revoked.", "success");
                      } catch (err) {
                        toast.show(
                          `Revoke failed: ${(err as Error).message ?? String(err)}`,
                          "error",
                        );
                      }
                    }}
                    data-testid={`revoke-delegate-${ulid}`}
                  >
                    Revoke
                  </button>
                ) : null}
              </div>
            </div>
          );
        })
      )}

      <IssueDelegateModal
        open={creating}
        onClose={() => setCreating(false)}
        onCreated={(label) => {
          setCreating(false);
          toast.show(`Delegate access issued to ${label}.`, "success");
        }}
      />
    </div>
  );
}

// Sensitivity classes that default-deny per the brief — the user can
// flip any back on in the form.
const DEFAULT_DENY: { id: string; label: string }[] = [
  { id: "mental_health", label: "Mental health" },
  { id: "substance_use", label: "Substance use" },
  { id: "sexual_health", label: "Sexual health" },
  { id: "reproductive", label: "Reproductive" },
];

const READABLE_EVENT_TYPES = [
  { id: "std.blood_glucose", label: "Glucose" },
  { id: "std.heart_rate_resting", label: "Heart rate" },
  { id: "std.body_temperature", label: "Temperature" },
  { id: "std.blood_pressure", label: "Blood pressure" },
  { id: "std.medication_dose", label: "Medications" },
  { id: "std.symptom", label: "Symptoms" },
  { id: "std.meal", label: "Meals" },
  { id: "std.mood", label: "Mood" },
  { id: "std.clinical_note", label: "Clinical notes" },
];

const WRITABLE_EVENT_TYPES = [
  { id: "std.blood_glucose", label: "Glucose" },
  { id: "std.medication_dose", label: "Medications" },
  { id: "std.symptom", label: "Symptoms" },
  { id: "std.meal", label: "Meals" },
  { id: "std.mood", label: "Mood" },
  { id: "std.clinical_note", label: "Clinical notes" },
];

type ExpiryChoice = "1m" | "3m" | "1y" | "custom";

function expiryToMs(choice: ExpiryChoice, customDays: number): number | null {
  const day = 86_400_000;
  switch (choice) {
    case "1m":
      return Date.now() + 30 * day;
    case "3m":
      return Date.now() + 90 * day;
    case "1y":
      return Date.now() + 365 * day;
    case "custom":
      return Date.now() + customDays * day;
  }
}

function IssueDelegateModal({
  open,
  onClose,
  onCreated,
}: {
  open: boolean;
  onClose: () => void;
  onCreated: (label: string) => void;
}) {
  const [label, setLabel] = useState("");
  const [identityBlob, setIdentityBlob] = useState("");
  const [readScope, setReadScope] = useState<Set<string>>(
    new Set(READABLE_EVENT_TYPES.map((t) => t.id)),
  );
  const [writeScope, setWriteScope] = useState<Set<string>>(new Set());
  const [denySensitivity, setDenySensitivity] = useState<Set<string>>(
    new Set(DEFAULT_DENY.map((t) => t.id)),
  );
  const [expiryChoice, setExpiryChoice] = useState<ExpiryChoice>("3m");
  const [customDays, setCustomDays] = useState(180);
  const [submitting, setSubmitting] = useState(false);
  const toast = useToast();

  if (!open) return null;

  const toggle = (
    set: Set<string>,
    setter: (s: Set<string>) => void,
    id: string,
  ) => {
    const next = new Set(set);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setter(next);
  };

  return (
    <Modal
      open={open}
      onClose={onClose}
      title="Issue delegate access"
      subtitle="Allow someone to act on your behalf — caregiver, family, spouse during incapacity."
      footer={
        <>
          <button className="btn" onClick={onClose} disabled={submitting}>
            Cancel
          </button>
          <button
            className="btn btn-accent"
            disabled={submitting || !label.trim()}
            onClick={async () => {
              setSubmitting(true);
              try {
                await createGrant({
                  granteeLabel: label.trim(),
                  granteeKind: "delegate",
                  purpose:
                    identityBlob.trim().length > 0
                      ? `delegate_identity:${identityBlob.trim()}`
                      : undefined,
                  defaultAction: "allow",
                  approvalMode: writeScope.size > 0 ? "always" : "auto_for_event_types",
                  expiresAtMs: expiryToMs(expiryChoice, customDays) ?? undefined,
                  notifyOnAccess: true,
                  stripNotes: false,
                  aggregationOnly: false,
                  readEventTypes: Array.from(readScope),
                  writeEventTypes: Array.from(writeScope),
                  denySensitivityClasses: Array.from(denySensitivity),
                });
                onCreated(label.trim());
                // Force re-bootstrap of grants list.
                void refreshStore();
              } catch (err) {
                toast.show(
                  `Issue failed: ${(err as Error).message ?? String(err)}`,
                  "error",
                );
              } finally {
                setSubmitting(false);
              }
            }}
          >
            {submitting ? "…" : "Issue delegate"}
          </button>
        </>
      }
    >
      <label className="field">
        Delegate label
        <input
          type="text"
          placeholder="e.g. Mom's caregiver"
          value={label}
          onChange={(e) => setLabel(e.target.value)}
          autoFocus
          data-testid="delegate-label-input"
        />
      </label>

      <label className="field">
        Delegate's OIDC identity (paste-token)
        <textarea
          placeholder="Optional. Paste the delegate's public OIDC identity blob — they'll share this with you out-of-band for v0. Full UX (QR / NFC accept) is v0.x."
          value={identityBlob}
          onChange={(e) => setIdentityBlob(e.target.value)}
          rows={3}
        />
        <span className="muted" style={{ fontSize: 11 }}>
          v0 limitation — the identity is recorded as a free-text purpose blob.
          The delegate-grant proto extension lands in v0.x and the field becomes
          structured.
        </span>
      </label>

      <fieldset className="field">
        <legend>Read scope (event types)</legend>
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 4 }}>
          {READABLE_EVENT_TYPES.map((t) => (
            <label key={t.id} className="check-row" style={{ fontSize: 13 }}>
              <input
                type="checkbox"
                checked={readScope.has(t.id)}
                onChange={() => toggle(readScope, setReadScope, t.id)}
              />{" "}
              {t.label}
            </label>
          ))}
        </div>
      </fieldset>

      <fieldset className="field">
        <legend>Write scope (event types — empty = read-only)</legend>
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 4 }}>
          {WRITABLE_EVENT_TYPES.map((t) => (
            <label key={t.id} className="check-row" style={{ fontSize: 13 }}>
              <input
                type="checkbox"
                checked={writeScope.has(t.id)}
                onChange={() => toggle(writeScope, setWriteScope, t.id)}
              />{" "}
              {t.label}
            </label>
          ))}
        </div>
        <span className="muted" style={{ fontSize: 11 }}>
          When write scope is non-empty, every submitted write goes through your
          Pending review queue (approval_mode=always).
        </span>
      </fieldset>

      <fieldset className="field">
        <legend>Sensitivity-class deny</legend>
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 4 }}>
          {DEFAULT_DENY.map((t) => (
            <label key={t.id} className="check-row" style={{ fontSize: 13 }}>
              <input
                type="checkbox"
                checked={denySensitivity.has(t.id)}
                onChange={() => toggle(denySensitivity, setDenySensitivity, t.id)}
              />{" "}
              Deny {t.label}
            </label>
          ))}
        </div>
        <span className="muted" style={{ fontSize: 11 }}>
          Defaults deny the four sensitive classes; uncheck to share. General-class
          (vitals, meds, allergies) is always allowed.
        </span>
      </fieldset>

      <label className="field">
        Expiry
        <select
          value={expiryChoice}
          onChange={(e) => setExpiryChoice(e.target.value as ExpiryChoice)}
        >
          <option value="1m">1 month</option>
          <option value="3m">3 months</option>
          <option value="1y">1 year</option>
          <option value="custom">Custom (days)</option>
        </select>
      </label>
      {expiryChoice === "custom" ? (
        <label className="field">
          Custom expiry (days)
          <input
            type="number"
            min={1}
            max={3650}
            value={customDays}
            onChange={(e) => setCustomDays(Number(e.target.value))}
          />
        </label>
      ) : null}

      <p className="muted" style={{ fontSize: 12 }}>
        Effective expiry: {fmtDays(((expiryToMs(expiryChoice, customDays) ?? Date.now()) - Date.now()) / 86_400_000)}
      </p>
    </Modal>
  );
}
