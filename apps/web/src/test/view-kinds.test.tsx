import "@testing-library/jest-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { FormView } from "../views/FormView";
import { BoardView } from "../views/BoardView";
import { CalendarView } from "../views/CalendarView";
import { GalleryView } from "../views/GalleryView";
import { apiClient, type AppView } from "../api";

const view = (kind: string, config: Record<string, unknown>): AppView => ({
  id: "v1", app_slug: "grants", table_id: "t", name: "A view", kind, ownership: "collaborative", owner: null,
  config, version: 1, created_by: "a", created_at: "", updated_by: "a", updated_at: "",
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

// ---------------------------------------------------------------------------------------------------------------------
describe("a form view", () => {
  const fields = [
    { name: "kind", label: "Kind", field_type: "Select", required: true, select_options: [{ id: "grant", label: "Grant" }, { id: "loan", label: "Loan" }] },
    { name: "amount", label: "Amount", field_type: "Number", required: true },
    { name: "note", label: "Note", field_type: "Text", required: false },
  ];
  const form = view("form", {
    form_fields: [
      { name: "kind", label: "What kind of help?" },
      { name: "amount", label: "How much?", help: "In dollars", show_when: [{ field: "kind", op: "eq", value: "grant" }] },
      { name: "note", label: "Tell us more", show_when: [{ field: "kind", op: "ne", value: "grant" }] },
    ],
    success_message: "Thank you. We have your request.",
  });
  const choose = (value: string) => fireEvent.change(screen.getByLabelText(/What kind of help\?/), { target: { value } });

  it("shows a field only while its conditions hold, as the person answers", () => {
    render(<FormView appSlug="grants" view={form} fields={fields} />);
    expect(screen.getByLabelText(/Tell us more/)).toBeInTheDocument();
    expect(screen.queryByLabelText(/How much\?/)).not.toBeInTheDocument();
    choose("grant");
    expect(screen.getByLabelText(/How much\?/)).toBeInTheDocument();
    expect(screen.getByText("In dollars")).toBeInTheDocument();
    expect(screen.queryByLabelText(/Tell us more/)).not.toBeInTheDocument();
  });

  // Test 1 of the brief, in the browser: a hidden required field is not required on submit.
  it("does not ask for a required field the conditions hide, and sends only what is shown", async () => {
    const send = vi.spyOn(apiClient, "submitForm").mockResolvedValue({ record: { id: "r1", data: {}, version: 1 }, success_message: "Thank you. We have your request." });
    render(<FormView appSlug="grants" view={form} fields={fields} />);
    choose("loan");
    fireEvent.change(screen.getByLabelText(/Tell us more/), { target: { value: "Emergency" } });
    fireEvent.click(screen.getByRole("button", { name: "Submit" }));
    await waitFor(() => expect(send).toHaveBeenCalledWith("grants", "v1", { kind: "loan", note: "Emergency" }));
    expect(await screen.findByText("Thank you. We have your request.")).toBeInTheDocument();
  });

  it("holds back a submit that leaves a shown, required field empty, and says which", async () => {
    const send = vi.spyOn(apiClient, "submitForm");
    render(<FormView appSlug="grants" view={form} fields={fields} />);
    choose("grant");
    fireEvent.click(screen.getByRole("button", { name: "Submit" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/How much\?.*required/i);
    expect(send).not.toHaveBeenCalled();
  });

  it("drops the answer to a field that became hidden, so a changed mind is not sent", async () => {
    const send = vi.spyOn(apiClient, "submitForm").mockResolvedValue({ record: { id: "r1", data: {}, version: 1 }, success_message: "Done" });
    render(<FormView appSlug="grants" view={form} fields={fields} />);
    choose("grant");
    fireEvent.change(screen.getByLabelText(/How much\?/), { target: { value: "500" } });
    choose("loan");
    fireEvent.click(screen.getByRole("button", { name: "Submit" }));
    await waitFor(() => expect(send).toHaveBeenCalledWith("grants", "v1", { kind: "loan" }));
  });

  it("shows the server's refusal and keeps the answers", async () => {
    vi.spyOn(apiClient, "submitForm").mockRejectedValue(new Error("'amount' is not shown for these answers, so it cannot be answered"));
    render(<FormView appSlug="grants" view={form} fields={fields} />);
    choose("loan");
    fireEvent.click(screen.getByRole("button", { name: "Submit" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/not shown for these answers/);
    expect((screen.getByLabelText(/What kind of help\?/) as HTMLSelectElement).value).toBe("loan");
  });
});

// ---------------------------------------------------------------------------------------------------------------------
describe("a board", () => {
  const options = [{ id: "applied", label: "Applied" }, { id: "offer", label: "Offer made" }];
  const board = view("kanban", { kanban_column_field: "stage", column_limits: { applied: 2 } });
  const card = (n: number, stage: string) => ({ id: `c${n}`, version: 3, data: { title: `Card ${n}`, stage } });
  const page = (stage: string, from: number, count: number, more: boolean) => ({
    view_id: "v1", records: Array.from({ length: count }, (_, i) => card(from + i, stage)), next_cursor: more ? `after-${from + count}` : null,
  });

  beforeEach(() => {
    vi.spyOn(apiClient, "runView").mockImplementation(async (_app: string, _id: string, opts: { groupPath?: string[]; cursor?: string; limit?: number } = {}) => {
      if (!opts.groupPath) return { view_id: "v1", groups: { level: 0, field: "stage", items: [{ key: "applied", count: 10_000 }, { key: "offer", count: 2 }], has_more: false } } as never;
      if (opts.groupPath[0] === "applied") return (opts.cursor ? page("applied", 51, 50, true) : page("applied", 1, 50, true)) as never;
      return page("offer", 9001, 2, false) as never;
    });
  });

  // Test 2 of the brief: only the first page of each column loads, and the counts are the aggregate's.
  it("loads fifty cards of a column, with its whole count, and Load more brings the next fifty", async () => {
    render(<BoardView appSlug="grants" view={board} columnField={{ name: "stage", options }} />);
    const applied = await screen.findByRole("region", { name: /Applied/ });
    expect(within(applied).getByText("10,000")).toBeInTheDocument();
    expect(within(applied).getAllByRole("listitem")).toHaveLength(50);
    expect(apiClient.runView).toHaveBeenCalledWith("grants", "v1", { groupPath: ["applied"], limit: 50 });
    fireEvent.click(within(applied).getByRole("button", { name: "Load more" }));
    await waitFor(() => expect(within(applied).getAllByRole("listitem")).toHaveLength(100));
    expect(apiClient.runView).toHaveBeenCalledWith("grants", "v1", { groupPath: ["applied"], limit: 50, cursor: "after-51" });
    const offer = screen.getByRole("region", { name: /Offer made/ });
    expect(within(offer).queryByRole("button", { name: "Load more" })).not.toBeInTheDocument();
  });

  it("warns, and does not block, when a column is over its limit", async () => {
    render(<BoardView appSlug="grants" view={board} columnField={{ name: "stage", options }} />);
    const applied = await screen.findByRole("region", { name: /Applied/ });
    expect(within(applied).getByText(/Over its limit of 2/)).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: /Offer made/ })).queryByText(/Over its limit/)).not.toBeInTheDocument();
  });

  // Test 3 of the brief: a keyboard move updates the record and announces the new column.
  it("moves a card with Alt and an arrow key, updates the record with its version, and says where it went", async () => {
    const update = vi.spyOn(apiClient, "updateRecord").mockResolvedValue({ id: "c1", data: {}, version: 4 });
    render(<BoardView appSlug="grants" view={board} columnField={{ name: "stage", options }} />);
    const applied = await screen.findByRole("region", { name: /Applied/ });
    const first = within(applied).getAllByRole("listitem")[0];
    fireEvent.keyDown(first, { key: "ArrowRight", altKey: true });
    await waitFor(() => expect(update).toHaveBeenCalledWith("grants", "c1", 3, { title: "Card 1", stage: "offer" }));
    expect(await screen.findByText("Moved to Offer made.")).toBeInTheDocument();
    const offer = screen.getByRole("region", { name: /Offer made/ });
    expect(within(offer).getByText("Card 1")).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: /Applied/ })).queryByText("Card 1")).not.toBeInTheDocument();
  });

  it("does nothing at the edge, and puts a card back when the update fails", async () => {
    const update = vi.spyOn(apiClient, "updateRecord").mockRejectedValue(new Error("Someone else changed this card first."));
    render(<BoardView appSlug="grants" view={board} columnField={{ name: "stage", options }} />);
    const applied = await screen.findByRole("region", { name: /Applied/ });
    const first = within(applied).getAllByRole("listitem")[0];
    fireEvent.keyDown(first, { key: "ArrowLeft", altKey: true });
    expect(update).not.toHaveBeenCalled();
    fireEvent.keyDown(first, { key: "ArrowRight", altKey: true });
    expect(await screen.findByRole("alert")).toHaveTextContent(/Someone else changed this card first/);
    expect(within(screen.getByRole("region", { name: /Applied/ })).getByText("Card 1")).toBeInTheDocument();
  });
});

// ---------------------------------------------------------------------------------------------------------------------
describe("a calendar", () => {
  const cal = view("calendar", { calendar_date_field: "due" });
  const events = (n: number) => Array.from({ length: n }, (_, i) => ({ id: `e${i}`, version: 1, data: { title: `Event ${i}`, due: `2026-03-${String((i % 28) + 1).padStart(2, "0")}T00:00:00Z` } }));
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-03-15T12:00:00Z"));
  });
  afterEach(() => vi.useRealTimers());

  // Test 4 of the brief, from the screen: the month is one request for its range, and the rest is a count.
  it("asks for the month's range, shows what comes back, and says how many more there are", async () => {
    const run = vi.spyOn(apiClient, "runView").mockResolvedValue({ view_id: "v1", records: events(500), calendar: { shown: 500, total: 2000, more: 1500 } } as never);
    render(<CalendarView appSlug="grants" view={cal} titleField="title" />);
    await screen.findByText("+1,500 more");
    expect(run).toHaveBeenCalledWith("grants", "v1", { from: "2026-03-01", to: "2026-03-31" });
    expect(screen.getByRole("heading", { name: "March 2026" })).toBeInTheDocument();
    expect(screen.getAllByText(/^Event \d+$/).length).toBeGreaterThan(0);
  });

  it("moves to the next month, and to a week or a day, each asking for its own range", async () => {
    const run = vi.spyOn(apiClient, "runView").mockResolvedValue({ view_id: "v1", records: [], calendar: { shown: 0, total: 0, more: 0 } } as never);
    render(<CalendarView appSlug="grants" view={cal} titleField="title" />);
    await waitFor(() => expect(run).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    await waitFor(() => expect(run).toHaveBeenLastCalledWith("grants", "v1", { from: "2026-04-01", to: "2026-04-30" }));
    fireEvent.click(screen.getByRole("button", { name: "Week" }));
    await waitFor(() => expect(run).toHaveBeenLastCalledWith("grants", "v1", { from: "2026-03-29", to: "2026-04-04" }));
    fireEvent.click(screen.getByRole("button", { name: "Day" }));
    await waitFor(() => expect(run).toHaveBeenLastCalledWith("grants", "v1", { from: "2026-03-29", to: "2026-03-29" }));
  });

  it("draws whole weeks, from the Sunday before the first to the Saturday after the last", async () => {
    // March 2026 starts on a Sunday and ends on a Tuesday, so the grid runs 1 March to 4 April. February 2026 is four whole weeks.
    vi.spyOn(apiClient, "runView").mockResolvedValue({ view_id: "v1", records: [], calendar: { shown: 0, total: 0, more: 0 } } as never);
    render(<CalendarView appSlug="grants" view={cal} titleField="title" />);
    await waitFor(() => expect(apiClient.runView).toHaveBeenCalled());
    expect(screen.getAllByRole("group")).toHaveLength(35);
    fireEvent.click(screen.getByRole("button", { name: "Previous" }));
    await waitFor(() => expect(screen.getByRole("heading", { name: "February 2026" })).toBeInTheDocument());
    expect(screen.getAllByRole("group")).toHaveLength(28);
  });

  it("opens the record when an event is chosen", async () => {
    vi.spyOn(apiClient, "runView").mockResolvedValue({ view_id: "v1", records: events(2), calendar: { shown: 2, total: 2, more: 0 } } as never);
    const open = vi.fn();
    render(<CalendarView appSlug="grants" view={cal} titleField="title" onOpenRecord={open} />);
    fireEvent.click(await screen.findByRole("button", { name: /Event 0/ }));
    expect(open).toHaveBeenCalledWith("e0");
    expect(screen.queryByText(/more$/)).not.toBeInTheDocument();
  });
});

// ---------------------------------------------------------------------------------------------------------------------
describe("a gallery", () => {
  const gal = view("gallery", { gallery_cover_field: "photos", gallery_card_fields: ["program"] });
  const records = [
    { id: "g1", version: 1, data: { name: "Ada Lovelace", program: "Math", photos: [{ ref_id: "f0", name: "notes.pdf", media_type: "application/pdf" }, { ref_id: "f1", name: "ada.png", media_type: "image/png" }, { ref_id: "f2", name: "later.png", media_type: "image/png" }] } },
    { id: "g2", version: 1, data: { name: "Grace Hopper", program: "CS", photos: [] } },
    { id: "g3", version: 1, data: { name: "Plain", program: "None" } },
  ];

  // Test 5 of the brief: the first image is the cover, and a record with none shows initials.
  it("shows the first image of the attachment field, and initials for a record with none", async () => {
    vi.spyOn(apiClient, "runView").mockResolvedValue({ view_id: "v1", records, next_cursor: null } as never);
    const file = vi.spyOn(apiClient, "fetchRecordFile").mockResolvedValue(new Blob(["x"], { type: "image/png" }));
    URL.createObjectURL = vi.fn(() => "blob:cover");
    URL.revokeObjectURL = vi.fn();
    render(<GalleryView appSlug="grants" view={gal} titleField="name" />);
    const ada = (await screen.findByText("Ada Lovelace")).closest("li") as HTMLElement;
    await waitFor(() => expect(within(ada).getByRole("img", { name: "ada.png" })).toHaveAttribute("src", "blob:cover"));
    expect(file).toHaveBeenCalledTimes(1);
    expect(file).toHaveBeenCalledWith("grants", "g1", "f1");
    const grace = screen.getByText("Grace Hopper").closest("li") as HTMLElement;
    expect(within(grace).getByText("GH")).toBeInTheDocument();
    expect(within(screen.getByText("Plain").closest("li") as HTMLElement).getByText("P")).toBeInTheDocument();
    expect(within(ada).getByText("Math")).toBeInTheDocument();
  });

  it("falls back to initials when the image cannot be loaded", async () => {
    vi.spyOn(apiClient, "runView").mockResolvedValue({ view_id: "v1", records: [records[0]], next_cursor: null } as never);
    vi.spyOn(apiClient, "fetchRecordFile").mockRejectedValue(new Error("The file is still being scanned"));
    render(<GalleryView appSlug="grants" view={gal} titleField="name" />);
    const ada = (await screen.findByText("Ada Lovelace")).closest("li") as HTMLElement;
    await waitFor(() => expect(within(ada).getByText("AL")).toBeInTheDocument());
  });
});

// An app page of kind View that names one of these views opens it.
describe("an app page that names a form", () => {
  it("shows the form, with the labels and options of the table", async () => {
    const app = {
      slug: "grants", title: "Grants", orgCode: "BIO", department: "Biology", customDomain: "", verified: false, hermCapability: "", cedsDomain: "",
      status: "Published", updatedAt: "", recordsCount: 0, workspaceId: "ws-1", collaborators: [],
      manifest: {
        slug: "grants", title: "Grants", description: "", organization_code: "BIO", department: "Biology", custom_domain_verified: false, views: [], ceds_mappings: {},
        tables: [{ id: "t", slug: "t", name: "t", primary_field: "kind", fields: [{ name: "kind", label: "Kind", field_type: "Select", required: true, select_options: [{ id: "grant", label: "Grant" }] }] }],
        pages: [{ id: "home", title: "Home", kind: { View: { view_id: "v1" } } }],
      },
    } as unknown as import("../types").RegisteredApp;
    vi.spyOn(apiClient, "listViews").mockResolvedValue({ views: [view("form", { form_fields: [{ name: "kind", label: "What kind of help?" }] })] });
    const { PublishedAppView } = await import("../PublishedAppView");
    render(<PublishedAppView app={app} />);
    expect(await screen.findByLabelText(/What kind of help\?/)).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Grant" })).toBeInTheDocument();
  });
});

void act;
