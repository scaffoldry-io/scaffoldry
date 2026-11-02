import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { PolicyLibrary, SourceCompare } from "../admin/PolicyLibrary";
import { apiClient, type AdminLibraryPolicy, type LibraryHealth, type SourceComparison } from "../api";

const PASSAGE = "Student records are kept for seven years after the student leaves.";

const policy: AdminLibraryPolicy = {
  id: "p1",
  key: "records",
  title: "Records retention",
  summary: "",
  organization_id: "u1",
  unit_name: "Registrar",
  owner: { Person: { eppn: "records.officer@state.edu" } },
  review_every_days: 365,
  last_reviewed_at: "2025-01-01T00:00:00Z",
  last_reviewed_by: null,
  due: true,
  status: "Active",
  rule_counts: { Published: 3, Draft: 1 },
  open_questions: 2,
};

const health: LibraryHealth = {
  no_published_rule: { count: 1, items: [{ policy_id: "p2", policy_key: "travel", title: "Travel", link: "/admin/library/p2" }] },
  guidance_only: { count: 12, items: [{ rule_id: "r1", rule_key: "retention", policy_id: "p1", policy_key: "records", why: "Nothing enforces it", link: "/admin/library/p1#rule-r1" }] },
  sources: { count: 1, items: [{ source_id: "s1", title: "Schedule", policy_id: "p1", policy_key: "records", problem: "unreachable", link: "/admin/library/p1#source-s1" }] },
  review: { count: 1, items: [{ policy_id: "p1", policy_key: "records", title: "Records retention", past_review: true, oldest_open_question_days: 45, link: "/admin/library/p1" }] },
};

describe("the Policy library section of the console", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("lists policies with owner, unit, rule counts, open questions and review due", async () => {
    vi.spyOn(apiClient, "listAdminLibraryPolicies").mockResolvedValue([policy]);
    render(<PolicyLibrary path="/admin/library" navigate={() => {}} />);
    const row = (await screen.findByText("Records retention")).closest("tr")!;
    expect(within(row).getByText("records.officer@state.edu")).toBeInTheDocument();
    expect(within(row).getByText("Registrar")).toBeInTheDocument();
    expect(within(row).getByText("3 published, 1 draft")).toBeInTheDocument();
    expect(within(row).getByText("2")).toBeInTheDocument();
    expect(within(row).getByText(/Review due/)).toBeInTheDocument();
  });

  it("adds a policy only with a reason", async () => {
    vi.spyOn(apiClient, "listAdminLibraryPolicies").mockResolvedValue([]);
    const create = vi.spyOn(apiClient, "createLibraryPolicy").mockResolvedValue(policy);
    render(<PolicyLibrary path="/admin/library" navigate={() => {}} />);
    fireEvent.click(await screen.findByRole("button", { name: "New policy" }));
    fireEvent.change(screen.getByLabelText("Key"), { target: { value: "records" } });
    fireEvent.change(screen.getByLabelText("Title"), { target: { value: "Records retention" } });
    fireEvent.change(screen.getByLabelText("Unit"), { target: { value: "u1" } });
    fireEvent.change(screen.getByLabelText("Owner"), { target: { value: "records.officer@state.edu" } });
    const save = screen.getByRole("button", { name: "Create policy" });
    expect(save).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Reason"), { target: { value: "The registrar asked" } });
    fireEvent.click(save);
    await waitFor(() =>
      expect(create).toHaveBeenCalledWith(
        expect.objectContaining({ key: "records", title: "Records retention", organization_id: "u1", owner: { Person: { eppn: "records.officer@state.edu" } }, reason: "The registrar asked" })
      )
    );
  });

  it("answers the four health questions with counts and links", async () => {
    vi.spyOn(apiClient, "libraryHealth").mockResolvedValue(health);
    const navigate = vi.fn();
    render(<PolicyLibrary path="/admin/library/health" navigate={navigate} />);
    expect(await screen.findByRole("heading", { name: /No published rule \(1\)/ })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: /Guidance only \(12\)/ })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: /Sources to look at \(1\)/ })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: /Reviews due \(1\)/ })).toBeInTheDocument();
    expect(screen.getByText(/unreachable/)).toBeInTheDocument();
    expect(screen.getByText(/a question open 45 days/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("link", { name: /travel/ }));
    expect(navigate).toHaveBeenCalledWith("/admin/library/p2");
  });
});

// 3. The side-by-side view marks the cited passage in the old text and shows the banner for supplied text.
describe("a changed source, side by side", () => {
  beforeEach(() => vi.restoreAllMocks());

  const comparison: SourceComparison = {
    old: { snapshot_id: "n1", taken_at: "2026-10-01T00:00:00Z", text_origin: "fetched", untrusted_text: `Intro. ${PASSAGE} Outro.` },
    new: { snapshot_id: "n2", taken_at: "2026-11-01T00:00:00Z", text_origin: "supplied", untrusted_text: "Intro. Student records are kept for ten years after the student leaves. Outro." },
    cited: [PASSAGE],
  };

  it("marks the cited passage in the old text and says the new text was supplied", async () => {
    vi.spyOn(apiClient, "compareLibrarySource").mockResolvedValue(comparison);
    const { container } = render(<SourceCompare sourceId="s1" />);
    const old = await screen.findByRole("region", { name: "Before" });
    const marks = old.querySelectorAll("mark");
    expect(marks).toHaveLength(1);
    expect(marks[0].textContent).toBe(PASSAGE);
    const after = screen.getByRole("region", { name: "After" });
    expect(after.querySelectorAll("mark")).toHaveLength(0);
    expect(within(after).getByText(/ten years/)).toBeInTheDocument();
    expect(screen.getByText(/This text was supplied by a person/)).toBeInTheDocument();
    // The text is shown as text, never as markup.
    expect(container.innerHTML).not.toContain("<script");
  });

  it("says when there is nothing earlier to compare", async () => {
    vi.spyOn(apiClient, "compareLibrarySource").mockResolvedValue({ old: null, new: comparison.new, cited: [] });
    render(<SourceCompare sourceId="s1" />);
    expect(await screen.findByText(/only one copy of this source/i)).toBeInTheDocument();
  });
});
