import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/react";
import { Policies } from "../library/Policies";
import { ErrorState } from "../ui";
import { ApiError, apiClient } from "../api";

const RULE = {
  id: "r-1",
  policy_id: "p-1",
  rule_key: "retention-seven-years",
  policy_key: "records-policy",
  policy_title: "Records retention",
  statement: "Student records are kept for seven years after the student leaves.",
  applies_to: "student records",
  last_reviewed_at: "2026-09-01T00:00:00Z",
  enforcement: "Enforced",
  enforced_by: [{ id: "b-1", target_kind: "guard_template", name: "Workspace guard: retention", state: "Enforced" }],
  warning: null,
};

const GUIDANCE = {
  ...RULE,
  id: "r-2",
  rule_key: "hold-pauses",
  statement: "A legal hold pauses destruction.",
  enforcement: "Guidance",
  enforced_by: [{ id: "b-2", target_kind: "guard_template", name: "guard template gone", state: "Missing" }],
  warning: "What enforced this rule is gone. It is guidance until the policy owner binds it again.",
};

function quiet() {
  vi.spyOn(apiClient, "library").mockResolvedValue({ rules: [RULE, GUIDANCE], next_cursor: null });
  vi.spyOn(apiClient, "libraryPolicy").mockResolvedValue({
    policy: { key: "records-policy", title: "Records retention", summary: "How long records are kept.", last_reviewed_at: "2026-09-01T00:00:00Z", organization_id: "u" },
    rules: [RULE],
    sources: [{ title: "Retention schedule", href: "https://registrar.example.edu/schedule", kind: "url", state: "Stored", last_checked_at: "2026-10-01T00:00:00Z" }],
  });
}

describe("Policies page", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  // 7. A rule is a sentence with its policy link and a chip. No control id without a title.
  it("shows a rule as a sentence with its policy as a link and a chip that reads Enforced or Guidance", async () => {
    quiet();
    const { container } = render(<Policies />);
    const rule = await screen.findByTestId("library-rule-r-1");
    expect(within(rule).getByText("Student records are kept for seven years after the student leaves.")).toBeInTheDocument();
    expect(within(rule).getByRole("button", { name: "Records retention" })).toBeInTheDocument();
    expect(within(rule).getByText("Enforced")).toBeInTheDocument();
    expect(within(rule).getByText(/Workspace guard: retention/)).toBeInTheDocument();
    const second = screen.getByTestId("library-rule-r-2");
    expect(within(second).getByText("Guidance")).toBeInTheDocument();
    expect(within(second).getByText(/guidance until the policy owner binds it again/)).toBeInTheDocument();
    const text = container.textContent ?? "";
    for (const code of ["retention-seven-years", "hold-pauses", "records-policy", "r-1", "guard_template"]) {
      expect(text).not.toContain(code);
    }
  });

  it("opens a policy from its link and lists its sources as links that open in a new tab", async () => {
    quiet();
    render(<Policies />);
    const rule = await screen.findByTestId("library-rule-r-1");
    fireEvent.click(within(rule).getByRole("button", { name: "Records retention" }));
    expect(await screen.findByText("How long records are kept.")).toBeInTheDocument();
    const link = screen.getByRole("link", { name: "Retention schedule" });
    expect(link).toHaveAttribute("href", "https://registrar.example.edu/schedule");
    expect(link).toHaveAttribute("target", "_blank");
    expect(link.getAttribute("rel")).toContain("noopener");
    expect(screen.getByText(/Last checked/)).toBeInTheDocument();
  });

  it("filters by enforcement and searches", async () => {
    quiet();
    const seen = vi.spyOn(apiClient, "library");
    render(<Policies />);
    await screen.findByTestId("library-rule-r-1");
    fireEvent.change(screen.getByLabelText("Enforcement"), { target: { value: "Guidance" } });
    await screen.findByTestId("library-rule-r-1");
    expect(seen).toHaveBeenLastCalledWith(expect.objectContaining({ enforcement: "Guidance" }));
    fireEvent.change(screen.getByLabelText("Search"), { target: { value: "hold" } });
    await screen.findByTestId("library-rule-r-1");
    expect(seen).toHaveBeenLastCalledWith(expect.objectContaining({ q: "hold" }));
  });

  it("links a denial to the rule behind its policy", () => {
    const denial = new ApiError(403, "You are not a member of this workspace.", { code: "forbidden", policy: { id: "policy11", description: "Members only.", rule: "/library/rules/access/restricted-labs" } });
    render(<ErrorState error={denial} />);
    const link = screen.getByRole("link", { name: /rule behind this/i });
    expect(link).toHaveAttribute("href", "/policies/access/restricted-labs");
  });
});
