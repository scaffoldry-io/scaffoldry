import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { TemplatePicker } from "../TemplatePicker";
import { apiClient, type AppTemplate } from "../api";

const intake: AppTemplate = {
  id: "t-1",
  name: "Student intake",
  description: "Collects a student's name and number",
  category: "Admissions",
  contents: ['Table "Students" has 3 field(s): name, note, ssn.', 'Page "Roster" reads Students: name, ssn.'],
  needs_classification: "FERPA Sensitive",
};

describe("TemplatePicker", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("offers templates by category, says what each holds, and what classification it needs", async () => {
    vi.spyOn(apiClient, "templates").mockResolvedValue({ templates: [intake] });
    render(<TemplatePicker workspaceId="ws-1" onClose={() => {}} />);
    expect(await screen.findByRole("heading", { name: "Admissions" })).toBeInTheDocument();
    expect(screen.getByText(/Page "Roster" reads Students/)).toBeInTheDocument();
    expect(screen.getByText(/FERPA Sensitive/)).toBeInTheDocument();
  });

  it("says the app is live only when the server approved it", async () => {
    vi.spyOn(apiClient, "templates").mockResolvedValue({ templates: [intake] });
    vi.spyOn(apiClient, "useTemplate").mockResolvedValue({ proposal_id: "p", app_slug: "student-intake", approved: true, checks: [] });
    render(<TemplatePicker workspaceId="ws-1" onClose={() => {}} />);
    fireEvent.click(await screen.findByRole("button", { name: "Use this template" }));
    expect(await screen.findByText(/is live as student-intake/)).toBeInTheDocument();
  });

  it("names the raised checks when a second person is needed", async () => {
    vi.spyOn(apiClient, "templates").mockResolvedValue({ templates: [intake] });
    vi.spyOn(apiClient, "useTemplate").mockResolvedValue({
      proposal_id: "p",
      app_slug: "x",
      approved: false,
      checks: [{ name: "sensitivity", outcome: "fail", messages: ["field 'ssn' is sensitive"] }],
    });
    render(<TemplatePicker workspaceId="ws-low" onClose={() => {}} />);
    fireEvent.click(await screen.findByRole("button", { name: "Use this template" }));
    await waitFor(() => expect(screen.getByText(/waiting for a second person.*sensitivity/)).toBeInTheDocument());
    expect(screen.queryByText(/is live as/)).not.toBeInTheDocument();
  });
});
