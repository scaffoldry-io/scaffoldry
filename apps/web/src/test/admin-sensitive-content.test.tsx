import "@testing-library/jest-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { SensitiveContent } from "../admin/SensitiveContent";
import { apiClient, type AdminFieldRow, type ContentFinding, type SensitivitySettings } from "../api";

const finding = (over: Partial<ContentFinding> = {}): ContentFinding => ({
  id: "f1", app_slug: "registrar", table_id: "notes", field: "body", detector_id: "us_ssn", category: "pii", source: "scan",
  match_count: 3, record_count: 2, sample_record_ids: ["rec-1", "rec-2"], status: "open", decided_by: null, decided_at: null, reason: null,
  first_seen: "2026-11-10T00:00:00Z", last_seen: "2026-11-14T00:00:00Z", ...over,
});

const settings = (): SensitivitySettings => ({
  categories: [{ id: "pci", name: "Payment card data", source: "PCI DSS", protected: true, detectors: ["payment_card"] }],
  detectors: [{ id: "us_ssn", kind: "us_ssn", action: "flag", enabled: true }, { id: "payment_card", kind: "payment_card", action: "flag", enabled: true }],
  presets: [{ id: "pci", name: "Payment card data", switched_on: true }, { id: "phi", name: "Protected health information", switched_on: false }],
});

const fieldRow = (over: Partial<AdminFieldRow> = {}): AdminFieldRow => ({
  app: "registrar", app_title: "Registrar", table: "cards", field: "card", label: "Card", type: "Text", manifest_flag: false, overlay_flag: false, overlay_note: null,
  effective: false, ceds_code: null, suggested: false, used_in: 0, used_in_parts: { views: 0, formulas: 0, pages: 0, rules: 0 }, ...over,
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("Sensitive content, findings", () => {
  it("lists open findings and opens a review with the sample records as links and the consequence in words", async () => {
    vi.spyOn(apiClient, "contentFindings").mockResolvedValue({ findings: [finding()], total: 1, next_cursor: null });
    render(<SensitiveContent />);
    const table = await screen.findByRole("table", { name: "Findings" });
    expect(within(table).getByText("us_ssn")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Review registrar notes body" }));
    const link = await screen.findByRole("link", { name: "rec-1" });
    expect(link).toHaveAttribute("href", "/app/registrar?record=rec-1");
    fireEvent.click(screen.getByRole("button", { name: "Confirm…" }));
    expect(screen.getByText("This field will be treated as sensitive. Exports and reads will be restricted.")).toBeInTheDocument();
  });

  it("confirms with a reason", async () => {
    vi.spyOn(apiClient, "contentFindings").mockResolvedValue({ findings: [finding()], total: 1, next_cursor: null });
    const decide = vi.spyOn(apiClient, "decideFinding").mockResolvedValue(undefined);
    render(<SensitiveContent />);
    fireEvent.click(await screen.findByRole("button", { name: "Review registrar notes body" }));
    fireEvent.click(await screen.findByRole("button", { name: "Confirm…" }));
    fireEvent.change(screen.getByRole("textbox", { name: /reason/i }), { target: { value: "It is a real SSN." } });
    fireEvent.click(screen.getAllByRole("button", { name: "Confirm body" }).at(-1) as HTMLElement);
    await waitFor(() => expect(decide).toHaveBeenCalledWith("f1", "confirm", "It is a real SSN."));
  });

  it("dismisses with a reason, and says the finding will not come back", async () => {
    vi.spyOn(apiClient, "contentFindings").mockResolvedValue({ findings: [finding()], total: 1, next_cursor: null });
    const decide = vi.spyOn(apiClient, "decideFinding").mockResolvedValue(undefined);
    render(<SensitiveContent />);
    fireEvent.click(await screen.findByRole("button", { name: "Review registrar notes body" }));
    fireEvent.click(await screen.findByRole("button", { name: "Dismiss…" }));
    expect(screen.getByText(/will not come back until its detector changes/)).toBeInTheDocument();
    fireEvent.change(screen.getByRole("textbox", { name: /reason/i }), { target: { value: "A test number." } });
    fireEvent.click(screen.getAllByRole("button", { name: "Dismiss body" }).at(-1) as HTMLElement);
    await waitFor(() => expect(decide).toHaveBeenCalledWith("f1", "dismiss", "A test number."));
  });

  it("asks the server for each filter and can queue a scan of every table", async () => {
    const get = vi.spyOn(apiClient, "contentFindings").mockResolvedValue({ findings: [], total: 0, next_cursor: null });
    const scan = vi.spyOn(apiClient, "scanContent").mockResolvedValue(undefined);
    render(<SensitiveContent />);
    expect(await screen.findByText("No findings")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Show"), { target: { value: "dismissed" } });
    await waitFor(() => expect(get).toHaveBeenLastCalledWith(expect.objectContaining({ status: "dismissed" })));
    fireEvent.click(screen.getByRole("button", { name: "Scan every table" }));
    await waitFor(() => expect(scan).toHaveBeenCalledWith({ all: true }));
    expect(await screen.findByText(/Scans are queued/)).toBeInTheDocument();
  });
});

describe("Sensitive content, categories", () => {
  const open = async () => {
    vi.spyOn(apiClient, "contentFindings").mockResolvedValue({ findings: [], total: 0, next_cursor: null });
    render(<SensitiveContent />);
    fireEvent.click(await screen.findByRole("tab", { name: "Categories" }));
  };

  it("shows the presets with Switch on only for one that is off, and each detector's stats", async () => {
    vi.spyOn(apiClient, "sensitivitySettings").mockResolvedValue(settings());
    vi.spyOn(apiClient, "detectorStats").mockResolvedValue([{ detector_id: "us_ssn", findings: 4, confirmed: 1, dismissed: 3, dismissed_share: 0.75 }]);
    const on = vi.spyOn(apiClient, "enableSensitivityPreset").mockResolvedValue(undefined);
    await open();
    const table = await screen.findByRole("table", { name: "Detectors" });
    expect(within(table).getByText("3 (75% of decided)")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Switch on Payment card data" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Switch on Protected health information" }));
    await waitFor(() => expect(on).toHaveBeenCalledWith("phi"));
  });

  it("shows the dismissed share and asks for a reason before an action becomes block", async () => {
    vi.spyOn(apiClient, "sensitivitySettings").mockResolvedValue(settings());
    vi.spyOn(apiClient, "detectorStats").mockResolvedValue([{ detector_id: "us_ssn", findings: 4, confirmed: 1, dismissed: 3, dismissed_share: 0.75 }]);
    const save = vi.spyOn(apiClient, "updateSetting").mockResolvedValue({ key: "sensitivity.detectors", value: [] });
    await open();
    fireEvent.change(await screen.findByLabelText("Action of us_ssn"), { target: { value: "block" } });
    expect(save).not.toHaveBeenCalled();
    expect(screen.getByText(/3 findings of us_ssn were dismissed, 75% of those decided/)).toBeInTheDocument();
    fireEvent.change(screen.getByRole("textbox", { name: /reason/i }), { target: { value: "Nearly all confirmed in the audit." } });
    fireEvent.click(screen.getAllByRole("button", { name: "Block with us_ssn" }).at(-1) as HTMLElement);
    await waitFor(() => expect(save).toHaveBeenCalled());
    const [key, list, reason] = save.mock.calls[0];
    expect(key).toBe("sensitivity.detectors");
    expect((list as { id: string; action: string }[]).find((d) => d.id === "us_ssn")?.action).toBe("block");
    expect(reason).toBe("Nearly all confirmed in the audit.");
  });

  it("changes a flag or warn action at once", async () => {
    vi.spyOn(apiClient, "sensitivitySettings").mockResolvedValue(settings());
    vi.spyOn(apiClient, "detectorStats").mockResolvedValue([]);
    const save = vi.spyOn(apiClient, "updateSetting").mockResolvedValue({ key: "sensitivity.detectors", value: [] });
    await open();
    fireEvent.change(await screen.findByLabelText("Action of payment_card"), { target: { value: "warn" } });
    await waitFor(() => expect(save).toHaveBeenCalled());
  });
});

describe("Sensitive content, authorized stores", () => {
  const open = async () => {
    vi.spyOn(apiClient, "contentFindings").mockResolvedValue({ findings: [], total: 0, next_cursor: null });
    vi.spyOn(apiClient, "sensitivitySettings").mockResolvedValue(settings());
    vi.spyOn(apiClient, "adminFields").mockResolvedValue({ fields: [fieldRow(), fieldRow({ field: "holder", label: "Holder" })], total: 2, next_cursor: null });
    render(<SensitiveContent />);
    fireEvent.click(await screen.findByRole("tab", { name: "Authorized stores" }));
  };

  it("says what each designation means and removes one with a reason", async () => {
    vi.spyOn(apiClient, "authorizedStores").mockResolvedValue([{ category: "pci", scope: "field", workspace_id: "", app_slug: "registrar", table_id: "cards", field: "card", reason: "Payments.", set_by: "a@x", set_at: "2026-11-14T00:00:00Z" }]);
    const remove = vi.spyOn(apiClient, "removeAuthorizedStore").mockResolvedValue(undefined);
    await open();
    expect(await screen.findByText("pci may live in field card of cards in registrar")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /^Remove pci may live/ }));
    expect(screen.getByText(/The labels stay until a person removes them/)).toBeInTheDocument();
    fireEvent.change(screen.getByRole("textbox", { name: /reason/i }), { target: { value: "No longer true." } });
    fireEvent.click(screen.getAllByRole("button", { name: /^Remove pci may live/ }).at(-1) as HTMLElement);
    await waitFor(() => expect(remove).toHaveBeenCalledWith(expect.objectContaining({ category: "pci", scope: "field", field: "card" }), "No longer true."));
  });

  it("adds a store by searching for a place, and says that its fields are labeled at once", async () => {
    vi.spyOn(apiClient, "authorizedStores").mockResolvedValue([]);
    const put = vi.spyOn(apiClient, "putAuthorizedStore").mockResolvedValue(undefined);
    await open();
    expect(await screen.findByText("No authorized stores")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    fireEvent.change(screen.getByLabelText("Category"), { target: { value: "pci" } });
    fireEvent.change(screen.getByLabelText("Find the place"), { target: { value: "holder" } });
    const place = screen.getByLabelText("Place") as HTMLSelectElement;
    expect(place.options).toHaveLength(1);
    fireEvent.change(place, { target: { value: "Registrar, cards, holder" } });
    expect(screen.getByText(/labeled pci at once/)).toBeInTheDocument();
    fireEvent.change(screen.getByRole("textbox", { name: /reason/i }), { target: { value: "Payments." }});
    fireEvent.click(screen.getAllByRole("button", { name: /^Designate/ }).at(-1) as HTMLElement);
    await waitFor(() => expect(put).toHaveBeenCalledWith({ category: "pci", scope: "field", app_slug: "registrar", table_id: "cards", field: "holder" }, "Payments."));
  });
});
