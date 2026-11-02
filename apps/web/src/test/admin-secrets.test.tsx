import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { Secrets } from "../admin/Secrets";
import { apiClient, ApiError, type SecretListing } from "../api";

const listed: SecretListing = {
  name: "registrar_hook",
  created_by: "jordan.lee@state.edu",
  created_at: "2026-11-02T04:00:00Z",
  rotated_at: null,
  references: 2,
};

// Integrations phase 1, test 10: the panel shows names and never a value, and Replace takes a new value in a
// write-only field.
describe("the Secrets panel", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("lists each secret by name, with who added it and what uses it, and no value", async () => {
    vi.spyOn(apiClient, "listSecrets").mockResolvedValue([listed]);
    const { container } = render(<Secrets />);
    expect(await screen.findByText("registrar_hook")).toBeInTheDocument();
    expect(screen.getByText(/jordan\.lee@state\.edu/)).toBeInTheDocument();
    expect(screen.getByText("Used by 2")).toBeInTheDocument();
    // Nothing on the page could hold a value: no field until Replace is chosen.
    expect(container.querySelectorAll("input, textarea")).toHaveLength(0);
  });

  it("replaces a value through a write-only field with a reason, and clears it after", async () => {
    vi.spyOn(apiClient, "listSecrets").mockResolvedValue([listed]);
    const put = vi.spyOn(apiClient, "putSecret").mockResolvedValue(undefined);
    render(<Secrets />);
    fireEvent.click(await screen.findByRole("button", { name: "Replace registrar_hook" }));
    const field = screen.getByLabelText("New value") as HTMLInputElement;
    expect(field.type).toBe("password");
    expect(field.value).toBe("");
    expect(field.getAttribute("autocomplete")).toBe("off");
    fireEvent.change(field, { target: { value: "new-signing-key" } });
    fireEvent.change(screen.getByLabelText("Reason"), { target: { value: "Rotated after the vendor's notice" } });
    fireEvent.click(screen.getByRole("button", { name: "Save new value" }));
    await waitFor(() => expect(put).toHaveBeenCalledWith("registrar_hook", "new-signing-key", "Rotated after the vendor's notice"));
    await waitFor(() => expect(screen.queryByLabelText("New value")).not.toBeInTheDocument());
    expect(screen.queryByDisplayValue("new-signing-key")).not.toBeInTheDocument();
  });

  it("adds a secret by name", async () => {
    vi.spyOn(apiClient, "listSecrets").mockResolvedValue([]);
    const put = vi.spyOn(apiClient, "putSecret").mockResolvedValue(undefined);
    render(<Secrets />);
    fireEvent.click(await screen.findByRole("button", { name: "Add secret" }));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "sis_password" } });
    fireEvent.change(screen.getByLabelText("Value"), { target: { value: "p4ss" } });
    fireEvent.change(screen.getByLabelText("Reason"), { target: { value: "Nightly feed" } });
    fireEvent.click(screen.getByRole("button", { name: "Save secret" }));
    await waitFor(() => expect(put).toHaveBeenCalledWith("sis_password", "p4ss", "Nightly feed"));
  });

  it("says what to do when the server has no master key", async () => {
    vi.spyOn(apiClient, "listSecrets").mockRejectedValue(new ApiError(503, "No master key", { code: "master_key_missing" }));
    render(<Secrets />);
    expect(await screen.findByText(/no master key/i)).toBeInTheDocument();
    expect(screen.getByText(/SCAFFOLDRY_MASTER_KEY_FILE/)).toBeInTheDocument();
  });

  it("shows why a secret in use cannot be deleted", async () => {
    vi.spyOn(apiClient, "listSecrets").mockResolvedValue([listed]);
    vi.spyOn(apiClient, "deleteSecret").mockRejectedValue(new ApiError(409, "The secret registrar_hook is in use by webhooks 7. Change those first", { code: "constraint_violated" }));
    render(<Secrets />);
    fireEvent.click(await screen.findByRole("button", { name: "Delete registrar_hook" }));
    fireEvent.change(screen.getByRole("textbox", { name: /reason/i }), { target: { value: "Retired" } });
    fireEvent.click(screen.getByRole("button", { name: /^Confirm|^Delete registrar_hook$/ }));
    expect(await screen.findByText(/in use by webhooks 7/)).toBeInTheDocument();
  });
});
