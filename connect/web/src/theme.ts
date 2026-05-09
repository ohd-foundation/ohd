// Theme management for the Connect web SPA.
//
// Three modes:
//   - "system" (default) — track `prefers-color-scheme: dark` reactively.
//   - "dark"             — force dark.
//   - "light"            — force light.
//
// Storage: `localStorage["ohd-connect-theme"]`. Persists across sessions
// (unlike the self-session token which lives in `sessionStorage`).
//
// The actual variable swap is driven by `[data-theme="dark"|"light"]` on
// the `<html>` element — the CSS in `index.css` already defines both
// palettes.
//
// `bootstrapTheme()` is called from `main.tsx` BEFORE React mounts so
// there's no flash of wrong theme on first paint. `setTheme()` is called
// from `AppearanceSettingsPage`.

export type ThemePreference = "system" | "dark" | "light";

export const THEME_STORAGE_KEY = "ohd-connect-theme";

const VALID: ReadonlyArray<ThemePreference> = ["system", "dark", "light"];

function isValidPreference(v: string): v is ThemePreference {
  return (VALID as readonly string[]).includes(v);
}

export function getSystemTheme(): "dark" | "light" {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
    return "dark"; // Connect's default per ux-design.md
  }
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

export function getStoredPreference(): ThemePreference {
  if (typeof window === "undefined") return "system";
  try {
    const raw = window.localStorage.getItem(THEME_STORAGE_KEY);
    if (raw && isValidPreference(raw)) return raw;
  } catch {
    // localStorage may throw in private-browsing edge cases; fall through.
  }
  return "system";
}

/** Resolve the effective theme — collapses "system" → "dark" | "light". */
export function resolveTheme(pref: ThemePreference): "dark" | "light" {
  return pref === "system" ? getSystemTheme() : pref;
}

/** Apply a resolved theme to `<html data-theme="…">`. */
export function applyTheme(resolved: "dark" | "light"): void {
  if (typeof document === "undefined") return;
  document.documentElement.dataset.theme = resolved;
}

/**
 * Read the persisted preference, resolve it, apply to `<html>`, and (when
 * preference is "system") wire a `matchMedia` listener so the page reacts
 * to OS-level changes immediately.
 *
 * Idempotent — listening twice is a no-op since `applyTheme` overwrites
 * the dataset attribute.
 */
let mediaListener: ((e: MediaQueryListEvent) => void) | null = null;
let mediaQuery: MediaQueryList | null = null;

export function bootstrapTheme(): void {
  const pref = getStoredPreference();
  applyTheme(resolveTheme(pref));
  // Detach previous listener (e.g. user just switched away from system).
  if (mediaQuery && mediaListener) {
    mediaQuery.removeEventListener("change", mediaListener);
    mediaQuery = null;
    mediaListener = null;
  }
  if (pref === "system" && typeof window !== "undefined" && typeof window.matchMedia === "function") {
    mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
    mediaListener = (e: MediaQueryListEvent) => {
      applyTheme(e.matches ? "dark" : "light");
    };
    mediaQuery.addEventListener("change", mediaListener);
  }
}

/** Persist the new preference, apply it, and rewire the media listener. */
export function setTheme(pref: ThemePreference): void {
  if (typeof window !== "undefined") {
    try {
      window.localStorage.setItem(THEME_STORAGE_KEY, pref);
    } catch {
      // ignore — preference is applied for the session even if not persisted
    }
  }
  applyTheme(resolveTheme(pref));
  // Re-run bootstrap to (re)wire / un-wire the system-listener as needed.
  bootstrapTheme();
}
