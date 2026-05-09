import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { App } from "../App";

describe("OHD Care v0 — smoke", () => {
  it("renders the roster with 5 patients", () => {
    render(
      <MemoryRouter initialEntries={["/roster"]}>
        <App />
      </MemoryRouter>,
    );

    // Brand mark in the top bar.
    expect(screen.getByText("OHD Care")).toBeInTheDocument();

    // Roster page is mounted.
    expect(screen.getByTestId("roster-page")).toBeInTheDocument();

    // Five patient cards.
    const cards = screen.getAllByTestId("roster-card");
    expect(cards).toHaveLength(5);
  });
});
