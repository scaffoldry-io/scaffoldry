import "@testing-library/jest-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { PolicyOscal } from "../admin/PolicyOscal";
import { ApiError, apiClient, type PolicyVersionRow } from "../api";

const draft: PolicyVersionRow = {
  id: "v2", version: 2, status: "Draft", note: "Staff stop exporting", created_by: "a@state.edu", created_at: "2026-11-19T10:00:00Z",
  activated_by: null, activated_at: null, source: "permit (principal, action, resource);",
};
const active: PolicyVersionRow = { ...draft, id: "v1", version: 1, status: "Active", created_by: "system", activated_by: "system", activated_at: "2026-11-01T00:00:00Z" };

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function setup() {
  vi.spyOn(apiClient, "adminPolicyActive").mockResolvedValue({
    version: active, source: "permit (principal, action, resource);",
    policies: [{ id: "policy0", key: "dept-read-write", effect: "permit", oscal: "ac-3", source: "", description: "Members read their department.", text: "permit (...)" }],
  });
  vi.spyOn(apiClient, "adminOscalRequirements").mockResolvedValue([
    { control_id: "ac-3", control_title: "Access Enforcement", catalog: "sp800-53r5", family_title: "Access Control", baselines: ["low", "moderate", "high"], statement: "Enforced by Cedar.", status: "implemented", policy_ids: ["policy0"], evidence_count: 3, latest_evidence: "2026-11-01T00:00:00Z", edited: false },
  ]);
  vi.spyOn(apiClient, "adminPolicyVersions").mockResolvedValue([draft, active]);
  vi.spyOn(apiClient, "adminPolicyVersion").mockResolvedValue(draft);
}

describe("Policy and OSCAL", () => {
  it("shows each control with its title, never bare", async () => {
    setup();
    render(<PolicyOscal />);
    expect(await screen.findByText("AC-3 · Access Enforcement")).toBeInTheDocument();
    expect(screen.getByText("dept-read-write")).toBeInTheDocument();
  });

  it("lists the failing cases by name and disables Activate", async () => {
    setup();
    vi.spyOn(apiClient, "runPolicyCases").mockResolvedValue({
      version: draft, passed: 15, failed: 1,
      results: [{ name: "Staff can export protected student records", expected: "Allow", actual: "Deny", pass: false, error: "" }],
    });
    render(<PolicyOscal />);
    fireEvent.click(await screen.findByRole("tab", { name: "Versions" }));
    fireEvent.click(await screen.findByRole("button", { name: "Open version 2" }));
    const activate = await screen.findByRole("button", { name: "Activate" });
    expect(activate).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Run cases" }));
    expect(await screen.findByText(/Staff can export protected student records: expected Allow, got Deny/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Activate" })).toBeDisabled();
  });

  it("enables Activate when every case passes", async () => {
    setup();
    vi.spyOn(apiClient, "runPolicyCases").mockResolvedValue({
      version: draft, passed: 1, failed: 0, results: [{ name: "Any", expected: "Allow", actual: "Allow", pass: true, error: "" }],
    });
    render(<PolicyOscal />);
    fireEvent.click(await screen.findByRole("tab", { name: "Versions" }));
    fireEvent.click(await screen.findByRole("button", { name: "Open version 2" }));
    fireEvent.click(await screen.findByRole("button", { name: "Run cases" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Activate" })).toBeEnabled());
  });

  it("shows the line and column of each problem in a new draft", async () => {
    setup();
    vi.spyOn(apiClient, "createPolicyDraft").mockRejectedValue(new ApiError(400, "The policy has 1 problem", { code: "invalid", fields: { "001": "line 3, column 8: unexpected token `action`" } }));
    render(<PolicyOscal />);
    fireEvent.click(await screen.findByRole("tab", { name: "Versions" }));
    fireEvent.click(await screen.findByRole("button", { name: "New draft" }));
    fireEvent.change(screen.getByLabelText("Policy source"), { target: { value: "forbid (principal action, resource);" } });
    fireEvent.click(screen.getByRole("button", { name: "Save draft" }));
    expect(await screen.findByText("line 3, column 8: unexpected token `action`")).toBeInTheDocument();
  });

  it("fills the control id and shows its title when one is chosen in the drawer", async () => {
    setup();
    const search = vi.spyOn(apiClient, "searchOscalCatalog").mockResolvedValue({
      rows: [{ control_id: "ac-2", control_title: "Account Management", catalog: "sp800-53r5", family_id: "ac", family_title: "Access Control", baselines: ["low", "moderate", "high"], has_requirement: false, status: null }],
      total: 1, next_cursor: null,
    });
    render(<PolicyOscal />);
    fireEvent.click(await screen.findByRole("tab", { name: "OSCAL requirements" }));
    fireEvent.click(await screen.findByRole("button", { name: "Add a requirement" }));
    fireEvent.change(screen.getByLabelText("Search controls"), { target: { value: "account" } });
    fireEvent.click(await screen.findByRole("button", { name: /AC-2 · Account Management/ }));
    expect(search).toHaveBeenCalled();
    expect(screen.getByLabelText("Control")).toHaveValue("AC-2 · Account Management");
    expect(screen.getByLabelText("Control")).toHaveAttribute("readonly");
  });

  it("states the coverage in words that match the numbers the server sent", async () => {
    setup();
    const none = { implemented: 0, partial: 0, planned: 0, alternative: 0, "not-applicable": 0, none: 0 };
    const coverage = vi.spyOn(apiClient, "oscalCoverage").mockResolvedValue({
      catalog: "sp800-53r5", baseline: "moderate", total: 5, with_evidence: 2,
      counts: { ...none, implemented: 3, planned: 0, none: 2 },
      families: [{ family_id: "ac", family_title: "Access Control", total: 5, counts: { ...none, implemented: 3, none: 2 } }],
    });
    vi.spyOn(apiClient, "oscalCoverageGaps").mockResolvedValue({
      rows: [{ control_id: "ac-3", control_title: "Access Enforcement", catalog: "sp800-53r5", family_id: "ac", family_title: "Access Control" }],
      total: 2, next_cursor: null,
    });
    render(<PolicyOscal />);
    fireEvent.click(await screen.findByRole("tab", { name: "Coverage" }));
    expect(await screen.findByText("3 of 5 controls in Moderate have an implemented requirement. 2 have none.")).toBeInTheDocument();
    expect(coverage).toHaveBeenCalledWith("sp800-53r5", "moderate");
    expect(screen.getByText("AC-3 · Access Enforcement")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add requirement for ac-3" })).toBeInTheDocument();
  });
});
