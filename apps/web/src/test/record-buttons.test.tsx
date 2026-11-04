import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { RecordButtons } from "../RecordButtons";
import { apiClient } from "../api";

// Field types phase 5: a button field shows its label, is disabled with the reason when the person may not press it,
// and pressing starts the rule.
describe("record buttons", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("shows each button, and a disabled one with its reason", async () => {
    vi.spyOn(apiClient, "listRecordButtons").mockResolvedValue([
      { field: "review", label: "Start review", enabled: true, reason: null },
      { field: "send", label: "Send", enabled: false, reason: "This record does not meet the button's conditions. It needs: status is Submitted." },
    ]);
    render(<RecordButtons appSlug="grants" recordId="r1" />);
    const review = await screen.findByRole("button", { name: "Start review" });
    expect(review).toBeEnabled();
    const send = screen.getByRole("button", { name: "Send" });
    expect(send).toBeDisabled();
    expect(screen.getByText(/It needs: status is Submitted/)).toBeInTheDocument();
  });

  it("presses a button and says what it started", async () => {
    vi.spyOn(apiClient, "listRecordButtons").mockResolvedValue([{ field: "review", label: "Start review", enabled: true, reason: null }]);
    const press = vi.spyOn(apiClient, "pressRecordButton").mockResolvedValue({ instance_id: "start-review:r1", started: true });
    const done = vi.fn();
    render(<RecordButtons appSlug="grants" recordId="r1" onPressed={done} />);
    fireEvent.click(await screen.findByRole("button", { name: "Start review" }));
    await waitFor(() => expect(press).toHaveBeenCalledWith("grants", "r1", "review"));
    expect(await screen.findByText("Started.")).toBeInTheDocument();
    expect(done).toHaveBeenCalled();
  });

  it("shows nothing when the record has no buttons", async () => {
    vi.spyOn(apiClient, "listRecordButtons").mockResolvedValue([]);
    const { container } = render(<RecordButtons appSlug="grants" recordId="r1" />);
    await waitFor(() => expect(apiClient.listRecordButtons).toHaveBeenCalled());
    expect(container.textContent).toBe("");
  });
});
