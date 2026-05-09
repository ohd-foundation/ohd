import { useState } from "react";
import { Modal } from "../components/Modal";
import { useToast } from "../components/Toast";
import { fmtDate, fmtDays } from "../util";
import {
  createGrantFromTemplate,
  GRANT_TEMPLATES,
  getSnapshot,
  revokeGrantById,
  type GrantTemplateId,
} from "../ohdc/store";
import { ulidToCrockford } from "../ohdc/client";
import { useStoreVersion } from "../ohdc/useStore";

/**
 * Grants tab — list active grants, create one from a template, revoke.
 *
 * Per the spec, grant creation is template-driven so the user picks a
 * meaningful default (primary doctor / specialist for one visit / spouse /
 * researcher / emergency break-glass) instead of fiddling with rules.
 *
 * Per-grant audit is TBD (storage's AuditQuery is stubbed); the UI shows a
 * "Audit (coming soon)" affordance with an explanation.
 */
export function GrantsPage() {
  useStoreVersion();
  const snap = getSnapshot();
  const [creating, setCreating] = useState(false);
  const [shareSheet, setShareSheet] = useState<{ token: string; shareUrl: string; label: string } | null>(null);
  const toast = useToast();

  return (
    <section data-testid="grants-page">
      <header className="page-header">
        <div>
          <h1>Grants</h1>
          <p>Who can read or write your data, under what scope, for how long.</p>
        </div>
        <button className="btn btn-accent" onClick={() => setCreating(true)} data-testid="create-grant">
          + New grant
        </button>
      </header>

      {snap.grants.length === 0 ? (
        <div className="empty">
          <p>No active grants. Tap <strong>New grant</strong> to issue one to a doctor, family member, or researcher.</p>
        </div>
      ) : (
        snap.grants.map((g) => {
          const ulid = ulidToCrockford(g.ulid?.bytes);
          const expiresAt = g.expiresAtMs ? Number(g.expiresAtMs) : null;
          const expired = expiresAt != null && expiresAt < Date.now();
          const expiringSoon = expiresAt != null && !expired && expiresAt - Date.now() < 7 * 86_400_000;
          return (
            <div key={ulid} className="card">
              <div className="card-title">
                <div>
                  <h3>{g.granteeLabel}</h3>
                  <div className="muted" style={{ fontSize: 12, marginTop: 2 }}>
                    <span className="mono">{ulid.slice(0, 8)}…</span> · kind <strong>{g.granteeKind}</strong> · approval <strong>{g.approvalMode}</strong>
                  </div>
                </div>
                <div style={{ display: "flex", gap: 8 }}>
                  {expired ? (
                    <span className="flag flag-warn">expired</span>
                  ) : expiringSoon ? (
                    <span className="flag flag-warn">expiring soon</span>
                  ) : (
                    <span className="flag flag-success">active</span>
                  )}
                </div>
              </div>

              <dl className="kv-grid">
                <dt>Created</dt>
                <dd>{fmtDate(Number(g.createdAtMs))}</dd>
                <dt>Expires</dt>
                <dd>{expiresAt ? fmtDate(expiresAt) : "indefinite"}</dd>
                <dt>Read scope</dt>
                <dd>{g.eventTypeRules.filter((r) => r.effect === "allow").map((r) => r.eventType).join(", ") || "(default)"}</dd>
                <dt>Write scope</dt>
                <dd>{g.writeEventTypeRules.filter((r) => r.effect === "allow").map((r) => r.eventType).join(", ") || "(none)"}</dd>
                <dt>Last used</dt>
                <dd>{g.lastUsedMs ? fmtDate(Number(g.lastUsedMs)) : "never"}</dd>
                <dt>Use count</dt>
                <dd className="mono">{Number(g.useCount)}</dd>
              </dl>

              <div style={{ display: "flex", gap: 8, marginTop: 12, flexWrap: "wrap" }}>
                <button
                  className="btn btn-sm"
                  onClick={() => alert("Per-grant audit will be available once storage's AuditQuery RPC ships. Tracked in connect/web/STATUS.md.")}
                  type="button"
                >
                  View audit (TBD)
                </button>
                {!expired ? (
                  <button
                    className="btn btn-sm btn-danger"
                    type="button"
                    onClick={async () => {
                      if (!confirm(`Revoke grant for ${g.granteeLabel}? This is immediate and irreversible.`)) return;
                      try {
                        await revokeGrantById(ulid, "user_revoked");
                        toast.show("Grant revoked.", "success");
                      } catch (err) {
                        toast.show(`Revoke failed: ${(err as Error).message ?? String(err)}`, "error");
                      }
                    }}
                    data-testid={`revoke-${ulid}`}
                  >
                    Revoke
                  </button>
                ) : null}
              </div>
            </div>
          );
        })
      )}

      <CreateGrantModal
        open={creating}
        onClose={() => setCreating(false)}
        onCreated={(r) => {
          setCreating(false);
          setShareSheet(r);
        }}
      />

      <Modal
        open={!!shareSheet}
        onClose={() => setShareSheet(null)}
        title="Share this grant"
        subtitle={shareSheet ? `For ${shareSheet.label} — only shown once.` : undefined}
        footer={
          <button className="btn btn-primary" onClick={() => setShareSheet(null)}>
            Done
          </button>
        }
      >
        {shareSheet ? (
          <>
            <div className="banner warn">
              Save this artifact now — the grant token is only ever displayed once. Reveal of this token grants the recipient the configured scope.
            </div>
            <label className="field">
              Grant token
              <input className="mono" type="text" value={shareSheet.token} readOnly onClick={(e) => (e.target as HTMLInputElement).select()} />
            </label>
            <label className="field">
              Share URL
              <input className="mono" type="text" value={shareSheet.shareUrl} readOnly onClick={(e) => (e.target as HTMLInputElement).select()} />
            </label>
            <p className="muted" style={{ fontSize: 12 }}>
              Send via NFC tap, paste into the operator's Care app, or scan the QR code (TBD).
            </p>
          </>
        ) : null}
      </Modal>
    </section>
  );
}

function CreateGrantModal({
  open,
  onClose,
  onCreated,
}: {
  open: boolean;
  onClose: () => void;
  onCreated: (r: { token: string; shareUrl: string; label: string }) => void;
}) {
  const [template, setTemplate] = useState<GrantTemplateId>("primary_doctor");
  const [granteeLabel, setGranteeLabel] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const toast = useToast();

  if (!open) return null;
  const t = GRANT_TEMPLATES[template];

  return (
    <Modal
      open={open}
      onClose={onClose}
      title="Issue a new grant"
      subtitle="Pick a template; we set sensible defaults."
      footer={
        <>
          <button className="btn" onClick={onClose} disabled={submitting}>
            Cancel
          </button>
          <button
            className="btn btn-accent"
            disabled={submitting || !granteeLabel.trim()}
            onClick={async () => {
              setSubmitting(true);
              try {
                const r = await createGrantFromTemplate(template, granteeLabel.trim());
                toast.show(`Grant for ${granteeLabel} issued.`, "success");
                onCreated({ token: r.token, shareUrl: r.shareUrl, label: granteeLabel.trim() });
              } catch (err) {
                toast.show(`Create grant failed: ${(err as Error).message ?? String(err)}`, "error");
              } finally {
                setSubmitting(false);
              }
            }}
          >
            {submitting ? "…" : "Issue"}
          </button>
        </>
      }
    >
      <label className="field">
        Template
        <select value={template} onChange={(e) => setTemplate(e.target.value as GrantTemplateId)}>
          {(Object.keys(GRANT_TEMPLATES) as GrantTemplateId[]).map((k) => (
            <option key={k} value={k}>
              {GRANT_TEMPLATES[k].label}
            </option>
          ))}
        </select>
      </label>
      <p className="muted" style={{ margin: 0, fontSize: 12 }}>
        {t.sub}
      </p>

      <label className="field">
        Grantee label
        <input
          type="text"
          placeholder="e.g. Dr Eva Novák"
          value={granteeLabel}
          onChange={(e) => setGranteeLabel(e.target.value)}
          autoFocus
        />
      </label>

      <dl className="kv-grid" style={{ fontSize: 12 }}>
        <dt>Approval</dt>
        <dd>{t.approvalMode}</dd>
        <dt>Default action</dt>
        <dd>{t.defaultAction}</dd>
        <dt>Expires</dt>
        <dd>{fmtDays(t.expiresInDays)}</dd>
        <dt>Read scope</dt>
        <dd>{t.readEventTypes.join(", ") || "(none)"}</dd>
        <dt>Write scope</dt>
        <dd>{t.writeEventTypes.join(", ") || "(none)"}</dd>
        <dt>Strip notes</dt>
        <dd>{t.stripNotes ? "yes" : "no"}</dd>
        <dt>Aggregation only</dt>
        <dd>{t.aggregationOnly ? "yes" : "no"}</dd>
      </dl>
    </Modal>
  );
}
