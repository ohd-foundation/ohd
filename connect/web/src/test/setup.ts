// Vitest global setup — bring in jest-dom matchers (toBeInTheDocument, etc).
import "@testing-library/jest-dom/vitest";

// Smoke tests don't have a real OHD Storage server. The store layer falls
// through to a "no_token" snapshot when no token is set (sessionStorage is
// empty in jsdom unless we populate it). We don't bother stubbing fetch
// here; the bootstrap path short-circuits at `resolveSelfToken()`.
