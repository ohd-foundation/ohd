import { useEffect, useState } from "react";
import { useToast } from "../../components/Toast";
import {
  getStoredPreference,
  getSystemTheme,
  resolveTheme,
  setTheme,
  type ThemePreference,
} from "../../theme";

/**
 * Settings → Appearance. Three-way theme selector (System / Dark / Light)
 * persisted to localStorage and applied immediately to `<html>` via
 * `data-theme="…"`. The CSS in `index.css` already defines both palettes;
 * this page is the only UI surface that flips them.
 *
 * v0 deferred:
 *   - Density selector (compact / comfortable / spacious) — would need a
 *     `--space-*` variable swap in `index.css`. Out of scope for v0.x.
 *   - High-contrast mode for accessibility — landed when an a11y pass
 *     defines the variable values.
 */
export function AppearanceSettingsPage() {
  const toast = useToast();
  const [pref, setPref] = useState<ThemePreference>(() => getStoredPreference());
  const [systemTheme, setSystemTheme] = useState<"dark" | "light">(() => getSystemTheme());

  // Track OS theme so the "System (currently dark)" hint stays accurate.
  useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = (e: MediaQueryListEvent) => {
      setSystemTheme(e.matches ? "dark" : "light");
    };
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  const onPick = (next: ThemePreference) => {
    setPref(next);
    setTheme(next);
    toast.show(
      next === "system"
        ? `Following system (${getSystemTheme()}).`
        : `Switched to ${next} theme.`,
      "success",
    );
  };

  const effective = resolveTheme(pref);

  return (
    <div data-testid="settings-appearance">
      <div className="card">
        <div className="card-title">
          <h3>Theme</h3>
          <span className="flag flag-active">{effective}</span>
        </div>
        <p className="muted" style={{ marginTop: 0 }}>
          OHD Connect defaults to dark per <code>ux-design.md</code>. Pick "System" to
          follow your OS preference, or force light / dark explicitly.
        </p>

        <div
          role="radiogroup"
          aria-label="Theme preference"
          style={{ display: "flex", gap: 8, flexWrap: "wrap", marginTop: 8 }}
        >
          <ThemeOption
            value="system"
            current={pref}
            label="System"
            sub={`Currently ${systemTheme}`}
            onPick={onPick}
          />
          <ThemeOption value="dark" current={pref} label="Dark" sub="Default" onPick={onPick} />
          <ThemeOption value="light" current={pref} label="Light" sub="High-contrast" onPick={onPick} />
        </div>
      </div>

      <div className="card">
        <div className="card-title">
          <h3>About this setting</h3>
        </div>
        <p className="muted" style={{ marginTop: 0, fontSize: 13 }}>
          The theme preference lives in <code>localStorage</code> under{" "}
          <code>ohd-connect-theme</code>. It's per-browser, not per-account — your
          phone, tablet, and laptop can each prefer a different theme. To reset,
          pick "System".
        </p>
      </div>
    </div>
  );
}

function ThemeOption({
  value,
  current,
  label,
  sub,
  onPick,
}: {
  value: ThemePreference;
  current: ThemePreference;
  label: string;
  sub: string;
  onPick: (v: ThemePreference) => void;
}) {
  const checked = current === value;
  return (
    <label
      style={{
        flex: "1 1 140px",
        minWidth: 140,
        cursor: "pointer",
        padding: 12,
        border: `1px solid ${checked ? "var(--color-accent)" : "var(--color-border)"}`,
        borderRadius: "var(--radius-md)",
        background: checked ? "var(--color-accent-tint)" : "var(--color-surface)",
        display: "block",
      }}
    >
      <input
        type="radio"
        name="theme-pref"
        value={value}
        checked={checked}
        onChange={() => onPick(value)}
        data-testid={`theme-${value}`}
        style={{ marginRight: 6 }}
      />
      <strong style={{ fontWeight: 600 }}>{label}</strong>
      <div className="muted" style={{ fontSize: 12, marginTop: 2 }}>
        {sub}
      </div>
    </label>
  );
}
