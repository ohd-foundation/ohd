import { useEffect, useState } from "react";
import { useToast } from "../../components/Toast";

/**
 * Settings → Emergency / Break-glass.
 *
 * Mirrors the eight sections in `connect/spec/screens-emergency.md`:
 *   1. Feature toggle (default off; opt-in).
 *   2. Discovery — BLE beacon.
 *   3. Approval timing — timeout slider + default-on-timeout (Allow / Refuse).
 *   4. Lock-screen behaviour — full vs basic-info dialog.
 *   5. What responders see — history window, per-channel toggles, sensitivity classes.
 *   6. Location — share GPS opt-in.
 *   7. Trusted authorities — list + add/remove.
 *   8. Advanced — bystander-proxy role, reset to defaults.
 *
 * Persistence: this is a v0.1 form. Wiring is not yet hooked to the
 * storage `Settings.SetEmergencyConfig` RPC (storage doesn't ship that
 * surface yet — the server-side schema for the emergency-template grant
 * exists, the management RPC doesn't). For now the page persists settings
 * to `localStorage`. STATUS.md flags the swap point.
 */

const KEY = "ohd-connect-emergency-settings";

interface EmergencySettings {
  featureEnabled: boolean;
  bleBeacon: boolean;
  approvalTimeoutS: number;
  defaultOnTimeout: "allow" | "refuse";
  lockScreenMode: "full" | "basic_only";
  historyWindowH: 0 | 3 | 12 | 24;
  channels: {
    glucose: boolean;
    hr: boolean;
    bp: boolean;
    spo2: boolean;
    temperature: boolean;
    allergies: boolean;
    medications: boolean;
    blood_type: boolean;
    advance_directives: boolean;
    diagnoses: boolean;
  };
  sensitivity: {
    general: boolean;
    mental_health: boolean;
    substance_use: boolean;
    sexual_health: boolean;
    reproductive: boolean;
  };
  locationShare: boolean;
  bystanderProxy: boolean;
  trustRoots: { id: string; name: string; scope: string; removable: boolean }[];
}

const DEFAULTS: EmergencySettings = {
  featureEnabled: false,
  bleBeacon: true,
  approvalTimeoutS: 30,
  defaultOnTimeout: "allow",
  lockScreenMode: "full",
  historyWindowH: 24,
  channels: {
    glucose: true,
    hr: true,
    bp: true,
    spo2: true,
    temperature: true,
    allergies: true,
    medications: true,
    blood_type: true,
    advance_directives: true,
    diagnoses: true,
  },
  sensitivity: {
    general: true,
    mental_health: false,
    substance_use: false,
    sexual_health: false,
    reproductive: false,
  },
  locationShare: false,
  bystanderProxy: true,
  trustRoots: [
    { id: "ohd_default", name: "OHD Project (default root)", scope: "global", removable: false },
  ],
};

function loadSettings(): EmergencySettings {
  if (typeof window === "undefined") return DEFAULTS;
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return DEFAULTS;
    return { ...DEFAULTS, ...JSON.parse(raw) };
  } catch {
    return DEFAULTS;
  }
}

function saveSettings(s: EmergencySettings) {
  if (typeof window === "undefined") return;
  localStorage.setItem(KEY, JSON.stringify(s));
}

export function EmergencySettingsPage() {
  const [s, setS] = useState<EmergencySettings>(loadSettings);
  const toast = useToast();

  useEffect(() => {
    saveSettings(s);
  }, [s]);

  const update = <K extends keyof EmergencySettings>(k: K, v: EmergencySettings[K]) =>
    setS((prev) => ({ ...prev, [k]: v }));

  const disabled = !s.featureEnabled;

  return (
    <div data-testid="settings-emergency">
      <div className="banner info">
        Emergency settings are stored locally for v0.1 — the storage-side
        <code> Settings.SetEmergencyConfig </code> RPC ships in v0.x and will
        promote these to the per-user emergency-template grant. Patient-side
        only; the responder UX lives in the OHD Emergency component.
      </div>

      <Section title="Emergency access" sub="Let first responders see basic info about you in a medical emergency.">
        <Toggle
          checked={s.featureEnabled}
          onChange={(v) => update("featureEnabled", v)}
          title="Enable emergency access"
          sub="When enabled, your phone broadcasts a low-power Bluetooth signal so nearby emergency responders can find your OHD record. They cannot see anything until you (or a timeout) approves."
        />
      </Section>

      <Section title="Discovery" disabled={disabled}>
        <Toggle
          checked={s.bleBeacon}
          onChange={(v) => update("bleBeacon", v)}
          disabled={disabled}
          title="Bluetooth beacon"
          sub="Broadcasts an opaque ID. No health information leaves your phone via Bluetooth — the beacon only signals 'OHD installed here.' Battery cost is minimal."
        />
      </Section>

      <Section title="Approval timing" disabled={disabled}>
        <div className="toggle-row" aria-disabled={disabled}>
          <div className="copy">
            <span className="title">Approval timeout: {s.approvalTimeoutS}s</span>
            <span className="sub">
              When a first responder requests emergency access, you have this long to Approve or Reject. After the timeout, the action below applies automatically.
            </span>
          </div>
          <input
            type="range"
            min={10}
            max={300}
            step={5}
            value={s.approvalTimeoutS}
            disabled={disabled}
            onChange={(e) => update("approvalTimeoutS", Number(e.target.value))}
            style={{ width: 160 }}
          />
        </div>
        <div className="toggle-row" aria-disabled={disabled}>
          <div className="copy">
            <span className="title">If you don't respond before timeout</span>
            <span className="sub">
              <strong>Allow</strong>: better for unconscious users — responder gets emergency info if you can't react.{" "}
              <strong>Refuse</strong>: better against malicious requests when you're nearby and unaware.
            </span>
          </div>
          <select
            value={s.defaultOnTimeout}
            disabled={disabled}
            onChange={(e) => update("defaultOnTimeout", e.target.value as "allow" | "refuse")}
          >
            <option value="allow">Allow access</option>
            <option value="refuse">Refuse access</option>
          </select>
        </div>
      </Section>

      <Section title="Lock-screen behaviour" disabled={disabled}>
        <div className="toggle-row" aria-disabled={disabled}>
          <div className="copy">
            <span className="title">Approval dialog visibility</span>
            <span className="sub">
              <strong>Full dialog above lock screen</strong> (recommended): anyone who can pick up the phone can approve.{" "}
              <strong>Basic info only</strong>: hides the responder's name and request details until you unlock.
            </span>
          </div>
          <select
            value={s.lockScreenMode}
            disabled={disabled}
            onChange={(e) => update("lockScreenMode", e.target.value as "full" | "basic_only")}
          >
            <option value="full">Full dialog</option>
            <option value="basic_only">Basic info only</option>
          </select>
        </div>
      </Section>

      <Section title="What responders see" disabled={disabled}>
        <div className="toggle-row" aria-disabled={disabled}>
          <div className="copy">
            <span className="title">History window</span>
            <span className="sub">
              How much recent vital-signs history they can see. Even with 0h, they always get current values.
            </span>
          </div>
          <select
            value={s.historyWindowH}
            disabled={disabled}
            onChange={(e) => update("historyWindowH", Number(e.target.value) as 0 | 3 | 12 | 24)}
          >
            <option value={0}>0 hours</option>
            <option value={3}>3 hours</option>
            <option value={12}>12 hours</option>
            <option value={24}>24 hours</option>
          </select>
        </div>

        <div className="subsection">
          <h4>Per-channel toggles</h4>
          <p className="muted" style={{ fontSize: 12, margin: "0 0 8px" }}>
            Channels in your emergency profile. Tap a row to include or exclude that channel.
          </p>
          <Toggle
            checked={s.channels.allergies}
            onChange={(v) => update("channels", { ...s.channels, allergies: v })}
            disabled={disabled}
            title="Allergies"
            sub="Critical for safe drug administration."
          />
          <Toggle
            checked={s.channels.medications}
            onChange={(v) => update("channels", { ...s.channels, medications: v })}
            disabled={disabled}
            title="Active medications"
            sub="Drug interactions and current treatment context."
          />
          <Toggle
            checked={s.channels.blood_type}
            onChange={(v) => update("channels", { ...s.channels, blood_type: v })}
            disabled={disabled}
            title="Blood type"
            sub="Transfusion safety."
          />
          <Toggle
            checked={s.channels.advance_directives}
            onChange={(v) => update("channels", { ...s.channels, advance_directives: v })}
            disabled={disabled}
            title="Advance directives"
            sub="DNR, organ donation preferences."
          />
          <Toggle
            checked={s.channels.diagnoses}
            onChange={(v) => update("channels", { ...s.channels, diagnoses: v })}
            disabled={disabled}
            title="Active diagnoses"
            sub="Chronic conditions affecting treatment."
          />
          <Toggle
            checked={s.channels.glucose}
            onChange={(v) => update("channels", { ...s.channels, glucose: v })}
            disabled={disabled}
            title="Glucose readings"
            sub="Important for diabetic emergencies."
          />
          <Toggle
            checked={s.channels.hr}
            onChange={(v) => update("channels", { ...s.channels, hr: v })}
            disabled={disabled}
            title="Heart rate"
            sub="Recent HR for arrhythmia / shock assessment."
          />
          <Toggle
            checked={s.channels.bp}
            onChange={(v) => update("channels", { ...s.channels, bp: v })}
            disabled={disabled}
            title="Blood pressure"
            sub="Recent BP for cardiovascular context."
          />
          <Toggle
            checked={s.channels.spo2}
            onChange={(v) => update("channels", { ...s.channels, spo2: v })}
            disabled={disabled}
            title="SpO₂"
            sub="Oxygen saturation for respiratory emergencies."
          />
          <Toggle
            checked={s.channels.temperature}
            onChange={(v) => update("channels", { ...s.channels, temperature: v })}
            disabled={disabled}
            title="Temperature"
            sub="Fever / hypothermia assessment."
          />
        </div>

        <div className="subsection">
          <h4>Sensitivity classes</h4>
          <p className="muted" style={{ fontSize: 12, margin: "0 0 8px" }}>
            Higher-stakes data classes. Defaults are conservative — only general info is shared by default.
          </p>
          <Toggle
            checked={s.sensitivity.general}
            onChange={(v) => update("sensitivity", { ...s.sensitivity, general: v })}
            disabled={disabled}
            title="General"
            sub="Vitals, medications, allergies — typical emergency info. Default ON."
          />
          <Toggle
            checked={s.sensitivity.mental_health}
            onChange={(v) => update("sensitivity", { ...s.sensitivity, mental_health: v })}
            disabled={disabled}
            title="Mental health"
            sub="Diagnoses, prescriptions. Default OFF — enable if you'd want responders to know."
          />
          <Toggle
            checked={s.sensitivity.substance_use}
            onChange={(v) => update("sensitivity", { ...s.sensitivity, substance_use: v })}
            disabled={disabled}
            title="Substance use"
            sub="Default OFF — relevant for overdose / interaction context."
          />
          <Toggle
            checked={s.sensitivity.sexual_health}
            onChange={(v) => update("sensitivity", { ...s.sensitivity, sexual_health: v })}
            disabled={disabled}
            title="Sexual health"
            sub="Default OFF."
          />
          <Toggle
            checked={s.sensitivity.reproductive}
            onChange={(v) => update("sensitivity", { ...s.sensitivity, reproductive: v })}
            disabled={disabled}
            title="Reproductive"
            sub="Some emergencies need reproductive context — consider enabling if pregnant or with body-anatomy concerns."
          />
        </div>
      </Section>

      <Section title="Location" disabled={disabled}>
        <Toggle
          checked={s.locationShare}
          onChange={(v) => update("locationShare", v)}
          disabled={disabled}
          title="Share location"
          sub="If enabled, your phone shares its current GPS coordinates with the responding emergency authority when access is granted. Useful for ambulance dispatch when you can't say where you are."
        />
      </Section>

      <Section title="Trusted authorities" disabled={disabled}>
        <p className="muted" style={{ marginTop: 0, fontSize: 12 }}>
          Only requests signed by a trusted authority root can trigger the emergency dialog. The OHD Project default root verifies regional EMS / hospital roots; advanced users can pin extra roots.
        </p>
        {s.trustRoots.map((r) => (
          <div className="toggle-row" key={r.id} aria-disabled={disabled}>
            <div className="copy">
              <span className="title">{r.name}</span>
              <span className="sub">scope: {r.scope}</span>
            </div>
            <button
              className="btn btn-sm btn-danger"
              type="button"
              disabled={disabled || !r.removable}
              onClick={() => {
                if (!confirm(`Remove trust root "${r.name}"?`)) return;
                update("trustRoots", s.trustRoots.filter((x) => x.id !== r.id));
              }}
            >
              {r.removable ? "Remove" : "Built-in"}
            </button>
          </div>
        ))}
        <button
          className="btn"
          type="button"
          disabled={disabled}
          onClick={() => {
            const name = prompt("Trust root name (e.g. 'EMS Prague Region')");
            if (!name) return;
            const id = `user_${Date.now().toString(36)}`;
            update("trustRoots", [...s.trustRoots, { id, name, scope: "user-added", removable: true }]);
            toast.show("Trust root added (cert validation runs in v0.x).", "success");
          }}
        >
          + Add trust root
        </button>
      </Section>

      <Section title="Advanced" disabled={disabled}>
        <Toggle
          checked={s.bystanderProxy}
          onChange={(v) => update("bystanderProxy", v)}
          disabled={disabled}
          title="Bystander-proxy role"
          sub="Your phone helps forward emergency requests for nearby OHD users who don't have internet. Your phone never sees their data — it just relays encrypted bytes. Disable to opt out of this Good-Samaritan behaviour."
        />
        <div style={{ display: "flex", gap: 8, marginTop: 12 }}>
          <button
            className="btn"
            type="button"
            disabled={disabled}
            onClick={() => {
              if (!confirm("Reset emergency profile to defaults?")) return;
              setS(DEFAULTS);
              toast.show("Reset to defaults.", "success");
            }}
          >
            Reset to defaults
          </button>
          <button
            className="btn btn-danger"
            type="button"
            onClick={() => {
              if (!confirm("Disable emergency feature entirely? Responders won't be able to find your record.")) return;
              update("featureEnabled", false);
              toast.show("Emergency feature disabled.", "success");
            }}
          >
            Disable emergency feature
          </button>
        </div>
      </Section>
    </div>
  );
}

function Section({ title, sub, children, disabled }: { title: string; sub?: string; children: React.ReactNode; disabled?: boolean }) {
  return (
    <div className="card" aria-disabled={disabled} style={{ opacity: disabled ? 0.85 : 1 }}>
      <div className="card-title">
        <h3>{title}</h3>
      </div>
      {sub ? <p className="muted" style={{ marginTop: -8, marginBottom: 12 }}>{sub}</p> : null}
      {children}
    </div>
  );
}

function Toggle({
  title,
  sub,
  checked,
  onChange,
  disabled,
}: {
  title: string;
  sub?: string;
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <label className={`toggle-row ${disabled ? "disabled" : ""}`.trim()}>
      <div className="copy">
        <span className="title">{title}</span>
        {sub ? <span className="sub">{sub}</span> : null}
      </div>
      <input
        type="checkbox"
        className="switch"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
        aria-label={title}
      />
    </label>
  );
}
