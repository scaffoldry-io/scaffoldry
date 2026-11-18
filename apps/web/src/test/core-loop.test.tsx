import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import "@testing-library/jest-dom";
import { MultiViewWorkspace } from "../MultiViewWorkspace";
import { StandaloneIntakeForm } from "../StandaloneIntakeForm";
import { AppBuilder } from "../AppBuilder";
import { registeredAppOf } from "../AdminDesk";
import { setAuthToken } from "../api";
import type { RegisteredApp } from "../types";

// The loop: create an app, add a record, edit it, reload and see it saved, open its history.
// These tests stand in for the server at the transport and check what the screens send and show.

type Call = { method: string; path: string; body?: any };
type Reply = { status?: number; body?: unknown };

function server(handler: (c: Call) => Reply | undefined) {
  const calls: Call[] = [];
  global.fetch = vi.fn().mockImplementation((url, options: RequestInit = {}) => {
    const path = String(url).replace(/^.*\/api\/v1/, "");
    const call: Call = { method: (options.method || "GET").toUpperCase(), path, body: options.body ? JSON.parse(String(options.body)) : undefined };
    calls.push(call);
    const reply = handler(call) ?? (call.method === "GET" && /\/records$/.test(path) ? { body: { records: [] } } : { status: 404, body: { error: "not found" } });
    const status = reply.status ?? 200;
    return Promise.resolve({ ok: status < 400, status, statusText: "", json: async () => reply.body });
  }) as unknown as typeof fetch;
  return calls;
}

const titleField = { name: "record_title", label: "Title / Summary", field_type: "Text", required: true, ferpa_sensitive: false };

/** What "Co-Build New App" makes: one table, one required field, no pages. */
function newApp(over: Record<string, unknown> = {}, fields: any[] = [titleField]): RegisteredApp {
  return {
    slug: "app-new", title: "Untitled Application", orgCode: "", department: "Biology", customDomain: "", verified: false,
    hermCapability: "", cedsDomain: "", status: "Draft", updatedAt: "", recordsCount: 0, workspaceId: "ws-1", collaborators: [],
    manifest: {
      slug: "app-new", title: "Untitled Application", description: "", organization_code: "", department: "Biology", custom_domain_verified: false,
      tables: [{ id: "main", name: "Records", slug: "records", fields, primary_field: fields[0]?.name }],
      relationships: [], pages: [],
      views: [{ id: "main-view", table_id: "main", title: "All records", view_type: "Grid", fields }],
      ceds_mappings: {}, version: 3, ...over,
    },
  } as unknown as RegisteredApp;
}

beforeEach(() => setAuthToken("tok"));

describe("1. adding a record in the grid", () => {
  const openGrid = (app = newApp()) => render(<MultiViewWorkspace app={app} onBack={() => {}} onOpenStudio={() => {}} />);

  it("asks for the required field first, then stores what the server returns", async () => {
    const calls = server((c) => {
      if (c.method === "POST" && c.path === "/apps/app-new/records") {
        return { status: 201, body: { id: "srv-77", version: 1, data: { _table_id: "main", record_title: c.body.data.record_title } } };
      }
    });
    openGrid();
    await waitFor(() => expect(calls.some((c) => c.method === "GET")).toBe(true));
    fireEvent.click(screen.getByRole("button", { name: /\+ Add Record/ }));
    // Nothing is sent until the person has filled the required field.
    expect(calls.filter((c) => c.method === "POST")).toHaveLength(0);
    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText(/Title \/ Summary/), { target: { value: "First proposal" } });
    fireEvent.click(within(dialog).getByRole("button", { name: /Save record/i }));
    await waitFor(() => expect(calls.filter((c) => c.method === "POST")).toHaveLength(1));
    expect(calls.find((c) => c.method === "POST")?.body).toEqual({ data: { _table_id: "main", record_title: "First proposal" } });
    expect(await screen.findByText("First proposal")).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("shows the server's field error inside the form and keeps the form open", async () => {
    server((c) => {
      if (c.method === "POST") return { status: 400, body: { error: "field 'record_title' violates required constraint", code: "constraint_violated", field: "record_title", constraint: "required" } };
    });
    openGrid();
    fireEvent.click(screen.getByRole("button", { name: /\+ Click to add a new record/ }));
    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText(/Title \/ Summary/), { target: { value: "  " } });
    fireEvent.click(within(dialog).getByRole("button", { name: /Save record/i }));
    expect(await within(dialog).findByText(/violates required constraint/)).toBeInTheDocument();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });

  it("creates directly when the table has no required field", async () => {
    const loose = [{ name: "note", label: "Note", field_type: "Text", required: false, ferpa_sensitive: false }];
    const calls = server((c) => {
      if (c.method === "POST") return { status: 201, body: { id: "srv-1", version: 1, data: { _table_id: "main", note: "" } } };
    });
    openGrid(newApp({}, loose));
    fireEvent.click(screen.getByRole("button", { name: /\+ Add Record/ }));
    await waitFor(() => expect(calls.filter((c) => c.method === "POST")).toHaveLength(1));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});

describe("2. the intake form stores the record", () => {
  const fill = () => {
    fireEvent.change(screen.getByTestId("field-input-record_title"), { target: { value: "From the form" } });
    fireEvent.click(screen.getByTestId("submit-intake-form-btn"));
  };

  it("posts through the records API and shows the id the server returned, with no invented hash", async () => {
    const calls = server((c) => {
      if (c.method === "POST") return { status: 201, body: { id: "9f2c-real-id", version: 1, data: { _table_id: "main", record_title: "From the form" } } };
    });
    const submitted = vi.fn();
    render(<StandaloneIntakeForm app={newApp()} onRecordSubmitted={submitted} />);
    fill();
    expect(await screen.findByTestId("intake-confirmation-card")).toHaveTextContent("9f2c-real-id");
    expect(calls.find((c) => c.method === "POST")?.path).toBe("/apps/app-new/records");
    expect(calls.find((c) => c.method === "POST")?.body).toEqual({ data: { _table_id: "main", record_title: "From the form" } });
    expect(document.body.textContent).not.toMatch(/REC-/);
    expect(document.body.textContent).not.toMatch(/sha256:/i);
    expect(document.body.textContent).not.toMatch(/attest/i);
    expect(submitted).toHaveBeenCalledWith("main", expect.objectContaining({ id: "9f2c-real-id" }));
  });

  it("shows the server's error and does not say the record was received", async () => {
    server((c) => {
      if (c.method === "POST") return { status: 403, body: { error: "You may not add records here." } };
    });
    render(<StandaloneIntakeForm app={newApp()} />);
    fill();
    expect(await screen.findByText(/You may not add records here/)).toBeInTheDocument();
    expect(screen.queryByTestId("intake-confirmation-card")).not.toBeInTheDocument();
    expect(screen.getByTestId("standalone-intake-form")).toBeInTheDocument();
  });
});

describe("3, 4, 5, 7. the studio is honest and can add plain fields", () => {
  const me = (c: Call): Reply | undefined => (c.path === "/auth/me" ? { body: { user: { eppn: "a@b.edu" } } } : undefined);
  const openStudio = (app = newApp()) => {
    server(me);
    return render(<AppBuilder app={app} onBack={() => {}} onOpenPublishedApp={() => {}} />);
  };

  it("shows no built-in sample content on a new app", () => {
    openStudio();
    for (const tab of ["pages", "data", "forms", "automations", "settings"]) {
      fireEvent.click(screen.getByTestId(`tab-btn-${tab}`));
      const text = document.body.textContent ?? "";
      expect(text).not.toMatch(/Executive Dashboard|2 CFR|3,450,000|\+14%|Workflow Kanban|Investigators|tbl-proposals|tbl-investigators|Research Proposals/);
    }
    fireEvent.click(screen.getByTestId("tab-btn-pages"));
    expect(screen.getByTestId("tab-btn-pages")).toHaveTextContent("(0 Pages)");
    expect(screen.getByText(/no pages yet/i)).toBeInTheDocument();
  });

  it("shows the app's own table, its field, and no relationships it does not have", () => {
    openStudio();
    fireEvent.click(screen.getByTestId("tab-btn-data"));
    expect(screen.getByTestId("tab-btn-data")).toHaveTextContent("(1 Tables)");
    expect(screen.getAllByText("Records").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Title / Summary").length).toBeGreaterThan(0);
    expect(document.body.textContent).not.toMatch(/Foreign Key Lattice/);
    expect(document.body.textContent).not.toMatch(/[1-9]\d* Linked Foreign Keys/);
  });

  it("has no Add Row, because rows are stored from the grid", () => {
    openStudio();
    fireEvent.click(screen.getByTestId("tab-btn-data"));
    expect(screen.queryByTestId("grid-add-row-btn")).not.toBeInTheDocument();
    expect(screen.queryByTestId("toolbar-add-record-btn")).not.toBeInTheDocument();
    expect(screen.getByText(/add records in the app's grid/i)).toBeInTheDocument();
  });

  it("adds a plain Date field and saves it to the server on Save Changes", async () => {
    const calls = server((c) => me(c) ?? (c.method === "PUT" && c.path === "/apps/app-new" ? { body: { ...c.body, version: 4 } } : undefined));
    const onSave = vi.fn();
    render(<AppBuilder app={newApp()} onBack={() => {}} onOpenPublishedApp={() => {}} onSaveApp={onSave} />);
    fireEvent.click(screen.getByTestId("tab-btn-data"));
    fireEvent.click(screen.getByTestId("add-column-header-btn"));
    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText(/^Label/), { target: { value: "Due date" } });
    fireEvent.change(within(dialog).getByLabelText(/^Type/), { target: { value: "Date" } });
    fireEvent.click(within(dialog).getByRole("button", { name: /Add field/i }));
    expect(screen.getAllByText("Due date").length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("button", { name: /Save Changes/ }));
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true));
    const put = calls.find((c) => c.method === "PUT")!;
    expect(put.path).toBe("/apps/app-new");
    expect(put.body.version).toBe(3);
    const saved = put.body.tables[0].fields.find((f: any) => f.name === "due_date");
    expect(saved).toMatchObject({ label: "Due date", field_type: "Date", required: false });
    // The grid reads the view's fields, so the view carries it too.
    expect(put.body.views[0].fields.map((f: any) => f.name)).toEqual(["record_title", "due_date"]);
    await waitFor(() => expect(onSave).toHaveBeenCalled());
    expect(onSave.mock.calls[0][0].manifest.version).toBe(4);
  });

  it("offers the plain field types", async () => {
    openStudio();
    fireEvent.click(screen.getByTestId("tab-btn-data"));
    fireEvent.click(screen.getByTestId("add-column-header-btn"));
    const dialog = await screen.findByRole("dialog");
    const options = within(dialog).getAllByRole("option").map((o) => (o as HTMLOptionElement).value);
    for (const t of ["Text", "Number", "Date", "Select"]) expect(options).toContain(t);
  });

  it("says plainly when the save fails and does not claim it saved", async () => {
    server((c) => me(c) ?? (c.method === "PUT" ? { status: 409, body: { error: "Someone else saved this app first. Reload it and try again.", code: "version_conflict" } } : undefined));
    render(<AppBuilder app={newApp()} onBack={() => {}} onOpenPublishedApp={() => {}} onSaveApp={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: /Save Changes/ }));
    expect(await screen.findByText(/not saved/i)).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/Saved to Sovereign Manifest/);
  });

  it("renames the app and saves the new title", async () => {
    const calls = server((c) => me(c) ?? (c.method === "PUT" ? { body: { ...c.body, version: 4 } } : undefined));
    render(<AppBuilder app={newApp()} onBack={() => {}} onOpenPublishedApp={() => {}} onSaveApp={vi.fn()} />);
    fireEvent.click(screen.getByTestId("tab-btn-settings"));
    fireEvent.change(screen.getByLabelText(/App name/i), { target: { value: "Grant tracker" } });
    fireEvent.click(screen.getByRole("button", { name: /Save Changes/ }));
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true));
    expect(calls.find((c) => c.method === "PUT")?.body.title).toBe("Grant tracker");
  });
});

describe("6. badges are shown only when true", () => {
  it("does not call a new app Governed", async () => {
    server(() => undefined);
    render(<MultiViewWorkspace app={newApp()} onBack={() => {}} onOpenStudio={() => {}} />);
    await waitFor(() => expect(screen.queryByText(/Loading records/i)).not.toBeInTheDocument());
    expect(screen.queryByText(/Governed/)).not.toBeInTheDocument();
  });

  it("reads an app with no status and no domain as a draft, and one with a domain as published", () => {
    const mine = { slug: "a", title: "A", description: "", department: "Biology", workspace_id: "ws-1", scope: "workspace" } as any;
    expect(registeredAppOf(mine, { tables: [], views: [] }).status).toBe("Draft");
    expect(registeredAppOf(mine, { tables: [], views: [], custom_domain: "a.example.edu" }).status).toBe("Published");
    expect(registeredAppOf(mine, { tables: [], views: [], status: "Collaborating" }).status).toBe("Collaborating");
  });
});
