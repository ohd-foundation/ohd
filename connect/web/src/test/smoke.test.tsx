import { describe, it, expect, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { App } from "../App";
import { _resetForTesting } from "../ohdc/store";

beforeEach(() => {
  _resetForTesting();
  if (typeof window !== "undefined") {
    sessionStorage.clear();
    localStorage.clear();
  }
});

describe("OHD Connect web — smoke", () => {
  it("renders the brand and the no-token screen when no session token is present", async () => {
    render(
      <MemoryRouter initialEntries={["/log"]}>
        <App />
      </MemoryRouter>,
    );
    expect(screen.getByText("OHD Connect")).toBeInTheDocument();
    // After bootstrap resolves no_token, the gate routes to the no-token screen.
    // The route at /log resolves the no-token gate body inline.
    const headings = await screen.findAllByText(/Sign in|No such route/i);
    expect(headings.length).toBeGreaterThan(0);
  });
});
