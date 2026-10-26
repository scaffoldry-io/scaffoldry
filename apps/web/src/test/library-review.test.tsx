import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent, within } from "@testing-library/react";
import { ReviewDrawer } from "../library/ReviewDrawer";
import { apiClient, type LibraryRuleReview } from "../api";

const PASSAGE = "Student records are kept for seven years after the student leaves.";

function review(overrides: Partial<LibraryRuleReview> = {}): LibraryRuleReview {
  return {
    rule: { id: "r-1", policy_id: "p-1", key: "retention", version: 1, statement: "Student records are kept for seven years.", applies_to: "student records", status: "InReview", submitted_by: "ada@state.edu", submitted_via: "person" },
    policy: { id: "p-1", key: "records", title: "Records retention" },
    citations: [{ snapshot_id: "s-1", passage: PASSAGE, locator: "section 1", before: "Section 1. ", after: " After that they are destroyed.", text_origin: "fetched", source_title: "Retention schedule" }],
    questions: [],
    previous: null,
    submitted_by: "ada@state.edu",
    submitted_by_name: "Ada Lovelace",
    submitted_via: "person",
    ...overrides,
  };
}

describe("ReviewDrawer", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("shows each cited passage in its context with the passage marked", async () => {
    vi.spyOn(apiClient, "libraryRuleReview").mockResolvedValue(review());
    render(<ReviewDrawer ruleId="r-1" onClose={() => {}} />);
    const cite = await screen.findByTestId("citation-0");
    expect(within(cite).getByText(PASSAGE).tagName).toBe("MARK");
    expect(cite).toHaveTextContent("Section 1. " + PASSAGE + " After that they are destroyed.");
    expect(within(cite).getByText(/Retention schedule/)).toBeInTheDocument();
    expect(screen.getByDisplayValue("Student records are kept for seven years.")).toBeInTheDocument();
  });

  it("warns when the text was typed in by a person", async () => {
    vi.spyOn(apiClient, "libraryRuleReview").mockResolvedValue(
      review({ citations: [{ snapshot_id: "s-1", passage: PASSAGE, locator: "", before: "", after: "", text_origin: "supplied", source_title: "Schedule" }] })
    );
    render(<ReviewDrawer ruleId="r-1" onClose={() => {}} />);
    expect(await screen.findByText("This text was typed in by a person. Check it against the original.")).toBeInTheDocument();
    expect(screen.queryByText(/Proposed by an AI tool/)).not.toBeInTheDocument();
  });

  it("says when an AI tool proposed the rule, and for whom", async () => {
    vi.spyOn(apiClient, "libraryRuleReview").mockResolvedValue(review({ submitted_via: "agent", rule: { ...review().rule, submitted_via: "agent" } }));
    render(<ReviewDrawer ruleId="r-1" onClose={() => {}} />);
    expect(await screen.findByText("Proposed by an AI tool used by Ada Lovelace.")).toBeInTheDocument();
    expect(screen.queryByText(/typed in by a person/)).not.toBeInTheDocument();
  });

  it("needs a note to publish or reject, and sends only a changed statement", async () => {
    vi.spyOn(apiClient, "libraryRuleReview").mockResolvedValue(review());
    const decide = vi.spyOn(apiClient, "libraryDecide").mockResolvedValue({ reworded: false, rule: review().rule });
    const done = vi.fn();
    render(<ReviewDrawer ruleId="r-1" onClose={() => {}} onDecided={done} />);
    const publish = await screen.findByRole("button", { name: "Publish" });
    const reject = screen.getByRole("button", { name: "Reject" });
    expect(publish).toBeDisabled();
    expect(reject).toBeDisabled();
    fireEvent.change(screen.getByLabelText(/note/i), { target: { value: "Matches the schedule" } });
    expect(publish).toBeEnabled();
    fireEvent.click(publish);
    await waitFor(() => expect(decide).toHaveBeenCalledWith("r-1", { decision: "publish", note: "Matches the schedule" }));
    await waitFor(() => expect(done).toHaveBeenCalled());
  });

  it("tells the reviewer a reworded rule needs a different reviewer", async () => {
    vi.spyOn(apiClient, "libraryRuleReview").mockResolvedValue(review());
    const decide = vi.spyOn(apiClient, "libraryDecide").mockResolvedValue({ reworded: true, rule: { ...review().rule, version: 2, submitted_by: "grace@state.edu" } });
    render(<ReviewDrawer ruleId="r-1" onClose={() => {}} />);
    const box = await screen.findByLabelText(/^statement/i, { selector: "textarea" });
    fireEvent.change(box, { target: { value: "Student records are kept seven years." } });
    fireEvent.change(screen.getByLabelText(/note/i), { target: { value: "Clearer" } });
    fireEvent.click(screen.getByRole("button", { name: "Publish" }));
    await waitFor(() => expect(decide).toHaveBeenCalledWith("r-1", { decision: "publish", note: "Clearer", statement: "Student records are kept seven years." }));
    expect(await screen.findByText(/saved as version 2\. A different reviewer must publish it\./)).toBeInTheDocument();
  });

  it("shows the published version it replaces and the questions with their answers", async () => {
    vi.spyOn(apiClient, "libraryRuleReview").mockResolvedValue(
      review({
        previous: { ...review().rule, version: 1, status: "Published", statement: "Records are kept six years." },
        questions: [{ id: "q-1", question: "Does a hold pause it?", answer: "Yes.", answered_by: "grace@state.edu" }, { id: "q-2", question: "Who holds the list?", answer: null, answered_by: null }],
      })
    );
    render(<ReviewDrawer ruleId="r-2" onClose={() => {}} />);
    expect(await screen.findByText("Records are kept six years.")).toBeInTheDocument();
    expect(screen.getByText("Does a hold pause it?")).toBeInTheDocument();
    expect(screen.getByText("Yes.")).toBeInTheDocument();
    expect(screen.getByText(/Who holds the list\?/)).toBeInTheDocument();
    expect(screen.getByText(/Not answered yet/)).toBeInTheDocument();
  });
});
