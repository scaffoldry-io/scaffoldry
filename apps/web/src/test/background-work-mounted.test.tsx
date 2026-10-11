import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom";
import AdminDesk from "../AdminDesk";

const running = {
  id: "j1",
  kind: "recalc_field",
  title: "Recalculating a field",
  description: "Writes a calculated field's values again after its definition changed.",
  state: "running",
  progress_done: 40,
  progress_total: 100,
};

function stubFetch(jobs: unknown[]) {
  global.fetch = vi.fn().mockImplementation((url) => {
    const body = String(url).includes("/jobs") ? { jobs } : [];
    return Promise.resolve({ ok: true, json: async () => body });
  });
}

describe("Background work on the desk", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
  });

  it("shows a signed-in person's running job with its plain name and progress", async () => {
    localStorage.setItem("scaffoldry_token", "test-token");
    stubFetch([running]);
    render(<AdminDesk initialPath="/" initialEppn="chair.physics@state.edu" />);

    const heading = await screen.findByText("Background work");
    const region = heading.closest('[role="status"]');
    expect(region).not.toBeNull();
    expect(region).toHaveTextContent("Recalculating a field");
    expect(region).toHaveTextContent("40%");
  });

  it("shows nothing when no job is queued or running", async () => {
    localStorage.setItem("scaffoldry_token", "test-token");
    stubFetch([{ ...running, state: "done" }]);
    render(<AdminDesk initialPath="/" initialEppn="chair.physics@state.edu" />);

    await waitFor(() => {
      const asked = (global.fetch as ReturnType<typeof vi.fn>).mock.calls.some(([u]) => String(u).includes("/jobs"));
      expect(asked).toBe(true);
    });
    expect(screen.queryByText("Background work")).not.toBeInTheDocument();
  });

  it("does not ask for jobs before anyone has signed in", async () => {
    stubFetch([running]);
    render(<AdminDesk initialPath="/" />);

    expect(await screen.findByTestId("token-sign-in")).toBeInTheDocument();
    expect(screen.queryByText("Background work")).not.toBeInTheDocument();
    const asked = (global.fetch as ReturnType<typeof vi.fn>).mock.calls.some(([u]) => String(u).includes("/jobs"));
    expect(asked).toBe(false);
  });
});
