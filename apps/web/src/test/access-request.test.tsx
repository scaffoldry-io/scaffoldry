import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { ApiError, apiClient } from "../api";
import { ErrorState } from "../ui";

const request = { object_kind: "app", object_id: "scholarships", role: "viewer", reason: "I need to read this app.", days: 14 };
const denied = new ApiError(403, "403 Forbidden: Cedar Policy restricts access to this app", { code: "forbidden", request });

describe("Asking for access", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("a refusal that carries a request offers Request access, and a refusal without one does not", () => {
    const { unmount } = render(<ErrorState error={denied} />);
    expect(screen.getByRole("button", { name: /request access/i })).toBeInTheDocument();
    unmount();
    render(<ErrorState error={new ApiError(403, "No", { code: "forbidden" })} />);
    expect(screen.queryByRole("button", { name: /request access/i })).not.toBeInTheDocument();
  });

  it("sends the request as the server filled it, with the person's own reason, and says what came of it", async () => {
    const file = vi.spyOn(apiClient, "fileAccessRequest").mockResolvedValue({ id: "r-1", status: "Approved", route: "auto", days: 14, waiting_on: [] });
    render(<ErrorState error={denied} />);
    fireEvent.click(screen.getByRole("button", { name: /request access/i }));
    const why = screen.getByLabelText(/why do you need it/i) as HTMLTextAreaElement;
    expect(why.value).toBe("I need to read this app.");
    fireEvent.change(why, { target: { value: "Quarterly audit" } });
    fireEvent.click(screen.getByRole("button", { name: /send request/i }));
    await waitFor(() => expect(file).toHaveBeenCalledWith({ ...request, reason: "Quarterly audit" }));
    expect(await screen.findByText(/you have access for 14 days/i)).toBeInTheDocument();
  });

  it("says who the request waits for when a person must decide", async () => {
    vi.spyOn(apiClient, "fileAccessRequest").mockResolvedValue({ id: "r-2", status: "Pending", route: "approver", days: 14, waiting_on: ["owner@state.edu"] });
    render(<ErrorState error={denied} />);
    fireEvent.click(screen.getByRole("button", { name: /request access/i }));
    fireEvent.click(screen.getByRole("button", { name: /send request/i }));
    expect(await screen.findByText(/waits for owner@state.edu/i)).toBeInTheDocument();
  });

  it("shows the reason it failed and keeps the form when filing is refused", async () => {
    vi.spyOn(apiClient, "fileAccessRequest").mockRejectedValue(new ApiError(400, "reason: reason may not be empty", { code: "bad_request" }));
    render(<ErrorState error={denied} />);
    fireEvent.click(screen.getByRole("button", { name: /request access/i }));
    fireEvent.click(screen.getByRole("button", { name: /send request/i }));
    expect(await screen.findByText(/reason may not be empty/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /send request/i })).toBeInTheDocument();
  });
});
