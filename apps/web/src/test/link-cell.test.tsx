import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { useState } from "react";
import { render, screen, waitFor, fireEvent, within } from "@testing-library/react";
import { LinkCell } from "../LinkCell";
import { RecordDetailDrawer } from "../RecordDetailDrawer";
import { DataGrid } from "../DataGrid";
import type { AppTable, FieldSpec } from "../types";

const programs = [
  { id: "p1", label: "CS", secondary: null },
  { id: "p2", label: "BIO", secondary: null },
  // A label that contains another is not an exact match for it.
  { id: "p3", label: "CS Minor", secondary: null },
];

/** Stubs `fetch` with the link-options endpoint: a contains-match on the label, as the server does. */
function stubLinkOptions() {
  const calls: string[] = [];
  const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const url = new URL(String(input), "http://x");
    calls.push(url.pathname + url.search);
    const q = (url.searchParams.get("q") ?? "").toLowerCase();
    const options = programs.filter((p) => p.label.toLowerCase().includes(q));
    return new Response(JSON.stringify({ options, next_cursor: null }), { status: 200, headers: { "Content-Type": "application/json" } });
  });
  vi.stubGlobal("fetch", fetchMock);
  return { fetchMock, calls };
}

function Cell() {
  const [value, setValue] = useState<{ id: string; label: string }[]>([]);
  return <LinkCell appSlug="demo" tableId="programs" label="Program" value={value} onChange={setValue} />;
}

describe("LinkCell", () => {
  beforeEach(() => vi.restoreAllMocks());
  afterEach(() => vi.unstubAllGlobals());

  it("pastes three labels, links the two that match exactly, and reports the one not found", async () => {
    stubLinkOptions();
    render(<Cell />);
    const cell = screen.getByRole("button", { name: "Program" });
    fireEvent.paste(cell, { clipboardData: { getData: () => "CS\nBIO\nChem" } });
    const banner = await screen.findByRole("status", { name: "Paste result" });
    expect(banner).toHaveTextContent('1 label was not linked: "Chem" was not found.');
    const chips = screen.getByRole("list", { name: "Linked records" });
    expect(within(chips).getByText("CS")).toBeInTheDocument();
    expect(within(chips).getByText("BIO")).toBeInTheDocument();
    expect(within(chips).queryByText("CS Minor")).not.toBeInTheDocument();
  });

  it("never guesses: a label that matches two records is reported, not linked", async () => {
    programs.push({ id: "p4", label: "BIO", secondary: "Second BIO" });
    try {
      stubLinkOptions();
      render(<Cell />);
      fireEvent.paste(screen.getByRole("button", { name: "Program" }), { clipboardData: { getData: () => "BIO" } });
      const banner = await screen.findByRole("status", { name: "Paste result" });
      expect(banner).toHaveTextContent('1 label was not linked: "BIO" matches 2 records.');
      expect(screen.queryByRole("list", { name: "Linked records" })).toBeEmptyDOMElement();
    } finally {
      programs.pop();
    }
  });

  it("is keyboard operable: Enter opens the picker, arrows move, Enter selects, Escape closes and returns focus", async () => {
    stubLinkOptions();
    render(<Cell />);
    const cell = screen.getByRole("button", { name: "Program" });
    cell.focus();
    fireEvent.keyDown(cell, { key: "Enter" });
    const box = await screen.findByRole("combobox");
    expect(box).toHaveFocus();
    await screen.findByText("3 results");
    fireEvent.keyDown(box, { key: "ArrowDown" });
    fireEvent.keyDown(box, { key: "Enter" });
    expect(within(screen.getByRole("list", { name: "Linked records" })).getByText("BIO")).toBeInTheDocument();
    fireEvent.keyDown(screen.getByRole("combobox"), { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("combobox")).not.toBeInTheDocument());
    expect(screen.getByRole("button", { name: "Program" })).toHaveFocus();
  });
});

describe("linked records in the drawer", () => {
  const programTable = {
    id: "programs",
    name: "Programs",
    slug: "programs",
    primary_field: "code",
    fields: [
      { name: "code", label: "Code", field_type: "Text", required: true, ferpa_sensitive: false },
      { name: "title", label: "Title", field_type: "Text", required: false, ferpa_sensitive: false },
      // The mirror: the same links seen from a program.
      { name: "applicants", label: "Applicants", field_type: "Link", required: false, ferpa_sensitive: false, link_target_table: "applications", link_related_field: "program" },
    ] as FieldSpec[],
    records: [
      { id: "p1", code: "CS", title: "Computer Science", applicants: ["a1"] },
      { id: "p2", code: "BIO", title: "Biology", applicants: ["a1"] },
    ],
  } as unknown as AppTable;
  const fields = [
    { name: "name", label: "Name", field_type: "Text", required: true, ferpa_sensitive: false },
    { name: "program", label: "Program", field_type: "Link", required: false, ferpa_sensitive: false, link_target_table: "programs", link_related_field: "applicants" },
  ] as FieldSpec[];

  function Drawer({ onUpdate }: { onUpdate: (id: string, field: string, value: unknown) => void }) {
    const [record, setRecord] = useState<Record<string, unknown>>({ id: "a1", name: "Ada", program: ["p1", "p2"] });
    const applications = { id: "applications", name: "Applications", slug: "applications", primary_field: "name", fields, records: [record] } as unknown as AppTable;
    return (
      <RecordDetailDrawer
        selectedRecordId="a1"
        onClose={() => {}}
        activeTable={applications}
        tables={[applications, programTable]}
        setTables={() => {}}
        appSlug="demo"
        onUpdateRecordField={(id, field, value) => {
          onUpdate(id, field, value);
          setRecord((r) => ({ ...r, [field]: value }));
        }}
        onDuplicateRecord={() => {}}
        onDeleteRecord={() => {}}
      />
    );
  }

  it("shows the target's fields as a mini table, and Unlink sends the update and removes the row", () => {
    const onUpdate = vi.fn();
    render(<Drawer onUpdate={onUpdate} />);
    const section = screen.getByRole("table", { name: "Program" });
    expect(within(section).getByRole("columnheader", { name: "Title" })).toBeInTheDocument();
    expect(within(section).getByText("Computer Science")).toBeInTheDocument();
    fireEvent.click(within(section).getByRole("button", { name: "Unlink CS" }));
    expect(onUpdate).toHaveBeenCalledWith("a1", "program", ["p2"]);
    expect(within(screen.getByRole("table", { name: "Program" })).queryByText("Computer Science")).not.toBeInTheDocument();
    expect(within(screen.getByRole("table", { name: "Program" })).getByText("Biology")).toBeInTheDocument();
  });

  it("shows the mirror field on the other table the same way", () => {
    const applications = { id: "applications", name: "Applications", slug: "applications", primary_field: "name", fields, records: [{ id: "a1", name: "Ada", program: ["p1"] }] } as unknown as AppTable;
    render(
      <RecordDetailDrawer
        selectedRecordId="p1"
        onClose={() => {}}
        activeTable={programTable}
        tables={[applications, programTable]}
        setTables={() => {}}
        appSlug="demo"
        onUpdateRecordField={() => {}}
        onDuplicateRecord={() => {}}
        onDeleteRecord={() => {}}
        onOpenRecord={() => {}}
      />,
    );
    const mirror = screen.getByRole("table", { name: "Applicants" });
    expect(within(mirror).getByText("Ada")).toBeInTheDocument();
    expect(within(mirror).getByRole("button", { name: "Open Ada" })).toBeInTheDocument();
    expect(within(mirror).getByRole("button", { name: "Unlink Ada" })).toBeInTheDocument();
  });
});

describe("a link column in the grid", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("shows chips by the target's primary field, and a paste in the cell links by label, not as text", async () => {
    stubLinkOptions();
    const onPatch = vi.fn();
    const target = { id: "programs", name: "Programs", slug: "programs", primary_field: "code", fields: [], records: [{ id: "p2", code: "BIO" }] } as unknown as AppTable;
    const fields = [
      { name: "name", label: "Name", field_type: "Text", required: true, ferpa_sensitive: false },
      { name: "program", label: "Program", field_type: "Link", required: false, ferpa_sensitive: false, link_target_table: "programs" },
    ] as FieldSpec[];
    render(<DataGrid appSlug="demo" fields={fields} records={[{ id: "a1", name: "Ada", program: ["p2"] }]} tables={[target]} onPatch={onPatch} />);
    const cell = screen.getByRole("button", { name: "Program" });
    expect(within(cell).getByText("BIO")).toBeInTheDocument();
    fireEvent.keyDown(cell, { key: "v", ctrlKey: true });
    fireEvent.paste(cell, { clipboardData: { getData: () => "CS" } });
    await waitFor(() => expect(onPatch).toHaveBeenCalledWith("a1", "program", ["p2", "p1"], expect.any(String)));
  });
});
