import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { RecordHistory, historySentence } from "../RecordHistory";
import { RecordDetailDrawer } from "../RecordDetailDrawer";
import * as api from "../api";
import { apiClient } from "../api";
import type { AppTable, FieldSpec } from "../types";

const fields = [
  { name: "status", label: "Status", field_type: "Text", required: false, ferpa_sensitive: false },
  { name: "title", label: "Title", field_type: "Text", required: false, ferpa_sensitive: false },
] as FieldSpec[];

const entry = {
  id: 9,
  record_id: "r1",
  table_id: "apps",
  version: 3,
  action: "update",
  changes: { status: { old: "Pending", new: "Admitted" } },
  actor: "rivera@state.edu",
  actor_name: "Dr. Rivera",
  actor_kind: "user",
  token_label: null,
  at: "2026-03-03T14:03:00Z",
};

describe("record history screens", () => {
  beforeEach(() => vi.restoreAllMocks());

  // 1. A history entry renders as a sentence with names, not ids.
  it("renders a change as a sentence with the person's name and the field's label", async () => {
    vi.spyOn(apiClient, "getRecordHistory").mockResolvedValue({ entries: [entry], next_cursor: null });
    render(<RecordHistory appSlug="adm" recordId="r1" fields={fields} canRestore={false} />);
    expect(await screen.findByText("Dr. Rivera changed Status from Pending to Admitted, 14:03.")).toBeInTheDocument();
    expect(screen.queryByText(/rivera@state\.edu/)).not.toBeInTheDocument();
    const agent = { ...entry, actor_kind: "agent", token_label: "admissions agent" };
    expect(historySentence(agent, fields)).toBe('Dr. Rivera, through "admissions agent", changed Status from Pending to Admitted, 14:03.');
  });

  // 2. `Restore this value` sends the restore request with the field and version.
  it("restores a value through a confirmation, naming the field and the version", async () => {
    vi.spyOn(apiClient, "getRecordHistory").mockResolvedValue({ entries: [entry], next_cursor: null });
    vi.spyOn(apiClient, "getRecord").mockResolvedValue({ id: "r1", data: {}, version: 5 });
    const restore = vi.spyOn(api, "restoreRecord").mockResolvedValue({});
    render(<RecordHistory appSlug="adm" recordId="r1" fields={fields} canRestore />);
    fireEvent.click(await screen.findByRole("button", { name: "Restore this value" }));
    fireEvent.change(screen.getByRole("textbox", { name: /reason/i }), { target: { value: "Entered in error" } });
    fireEvent.click(screen.getByRole("button", { name: "Restore Status" }));
    await waitFor(() => expect(restore).toHaveBeenCalledWith("adm", "r1", { to_version: 3, fields: ["status"], version: 5 }));
  });

  const table = { id: "apps", name: "Applications", slug: "apps", primary_field: "title", fields, records: [{ id: "r1", title: "A", status: "Admitted" }] } as unknown as AppTable;
  const drawer = (role: "owner" | "admin" | "editor" | "commenter" | "viewer", compliance = false) => (
    <RecordDetailDrawer
      selectedRecordId="r1"
      onClose={() => {}}
      activeTable={table}
      tables={[table]}
      setTables={() => {}}
      appSlug="adm"
      viewer={{ eppn: "me@state.edu", role, compliance }}
      onUpdateRecordField={() => {}}
      onDuplicateRecord={() => {}}
      onDeleteRecord={() => {}}
    />
  );

  // 3. A viewer sees no `Comments` composer. A commenter does.
  it("shows the comment composer to a commenter and not to a viewer, and says why", async () => {
    vi.spyOn(apiClient, "listComments").mockResolvedValue({ comments: [] });
    vi.spyOn(apiClient, "mentionable").mockResolvedValue({ people: [] });
    vi.spyOn(apiClient, "getRecordHold").mockResolvedValue(null);
    const { unmount } = render(drawer("viewer"));
    fireEvent.click(screen.getByRole("tab", { name: "Comments" }));
    expect(await screen.findByText("You can read comments. Your role cannot add them.")).toBeInTheDocument();
    expect(screen.queryByLabelText("Write a comment")).not.toBeInTheDocument();
    unmount();
    render(drawer("commenter"));
    fireEvent.click(screen.getByRole("tab", { name: "Comments" }));
    expect(await screen.findByLabelText("Write a comment")).toBeInTheDocument();
    expect(screen.getByText("As a commenter you can add comments. You cannot change the record.")).toBeInTheDocument();
  });

  // 4. `Disclosures` is absent for an editor.
  it("offers Disclosures to owners, admins and compliance, and not to an editor", async () => {
    vi.spyOn(apiClient, "getRecordHold").mockResolvedValue(null);
    vi.spyOn(apiClient, "getRecordDisclosures").mockResolvedValue({
      disclosures: [{ id: "d1", at: "2026-03-03T09:00:00Z", actor: "rivera@state.edu", actor_name: "Dr. Rivera", kind: "export", record_count: 340, fields: ["Student ID"], recipient: "Board of Regents" }],
    });
    const { unmount } = render(drawer("editor"));
    expect(screen.getByRole("tab", { name: "History" })).toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "Disclosures" })).not.toBeInTheDocument();
    unmount();
    render(drawer("owner"));
    fireEvent.click(screen.getByRole("tab", { name: "Disclosures" }));
    const list = await screen.findByRole("list", { name: "Disclosures" });
    expect(within(list).getByText("Exported 340 records including Student ID, by Dr. Rivera, 3 March.")).toBeInTheDocument();
    expect(within(list).getByText(/Board of Regents/)).toBeInTheDocument();
  });
});

import { AppActivity } from "../RecordHistory";

describe("the app's Activity feed", () => {
  it("lists every record's changes as sentences", async () => {
    vi.spyOn(apiClient, "getAppHistory").mockResolvedValue({ entries: [entry, { ...entry, id: 10, action: "create", changes: {} }], next_cursor: null });
    render(<AppActivity appSlug="adm" fields={fields} />);
    const list = await screen.findByRole("list", { name: "Activity" });
    expect(within(list).getByText("Dr. Rivera changed Status from Pending to Admitted, 14:03.")).toBeInTheDocument();
    expect(within(list).getByText("Dr. Rivera created the record, 14:03.")).toBeInTheDocument();
  });
});
