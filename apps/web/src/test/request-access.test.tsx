import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { setAuthToken, type AccessRequest } from "../api";
import { DatasetExplorer } from "../DatasetExplorer";
import { whereIsIt } from "../RequestAccessDrawer";

const entry = (over: Record<string, unknown> = {}) => ({
  id: "ds_1",
  dataset_id: "ds_1",
  kind: "table",
  name: "Course Enrollment",
  description: "Who enrolled",
  department: "Registrar",
  organization: "CHEM",
  sensitivity_level: "Internal",
  record_count: 12,
  size: { masked: false, label: "about 12 rows" },
  steward: { position_key: "chair", position: "Department chair" },
  request_fields: ["reason", "project", "duration"],
  fields: [{ name: "code", label: "Code", field_type: "Text", required: true, ferpa_sensitive: false }],
  ...over,
});

interface Sent {
  path: string;
  method: string;
  body: Record<string, unknown> | null;
}

/** A server that holds one catalog entry and answers a request for access with `reply`. */
function serve(reply: { status: number; body: unknown }, dataset = entry()) {
  const sent: Sent[] = [];
  global.fetch = vi.fn().mockImplementation((url, init?: RequestInit) => {
    const path = String(url).replace(/^.*\/api\/v1/, "");
    sent.push({ path, method: init?.method ?? "GET", body: init?.body ? JSON.parse(String(init.body)) : null });
    const r = path === "/access/requests" ? reply : { status: 200, body: { total: 1, truncated: false, datasets: [dataset], facets: { units: [], functions: [], categories: [] } } };
    return Promise.resolve({ ok: r.status < 400, status: r.status, statusText: "", json: async () => r.body });
  }) as unknown as typeof fetch;
  return sent;
}

const waiting: AccessRequest = {
  id: "r1",
  status: "Pending",
  route: "approver",
  days: 14,
  routing: { to: "steward", position: "chair", position_name: "Department chair", holders: ["chair@x.edu"], sentence: "Waiting for Department chair: chair@x.edu." },
};

async function open() {
  render(<DatasetExplorer />);
  await screen.findAllByText("Course Enrollment");
  fireEvent.click(screen.getByRole("button", { name: "Request access" }));
  return screen.findByRole("form", { name: "Request access" });
}

describe("Request access on a catalog entry", () => {
  beforeEach(() => setAuthToken("tok"));

  it("asks for the fields the dataset lists, and no others", async () => {
    serve({ status: 201, body: waiting });
    const form = await open();
    expect(form.textContent).toContain("Reason");
    expect(form.textContent).toContain("Project");
    expect(form.textContent).toContain("Days of access");
    expect(form.textContent).not.toContain("Agreement reference");
  });

  it("names a missing field and sends nothing", async () => {
    const sent = serve({ status: 201, body: waiting });
    await open();
    fireEvent.change(screen.getByLabelText(/Reason/), { target: { value: "Retention study" } });
    fireEvent.click(screen.getByRole("button", { name: "Send request" }));
    expect(await screen.findByText("Project is required")).toBeTruthy();
    expect(screen.getByText("Days of access is required")).toBeTruthy();
    expect(sent.some((s) => s.path === "/access/requests")).toBe(false);
  });

  it("sends the answers and then shows where the request is and who has it", async () => {
    const sent = serve({ status: 201, body: waiting });
    await open();
    fireEvent.change(screen.getByLabelText(/Reason/), { target: { value: "Retention study" } });
    fireEvent.change(screen.getByLabelText(/Project/), { target: { value: "Persistence 2027" } });
    fireEvent.change(screen.getByLabelText(/Days of access/), { target: { value: "14" } });
    fireEvent.click(screen.getByRole("button", { name: "Send request" }));
    const status = await screen.findByRole("status", { name: "Where your request is" });
    expect(status.textContent).toContain("Pending");
    expect(status.textContent).toContain("Waiting for Department chair: chair@x.edu.");
    const filed = sent.find((s) => s.path === "/access/requests");
    expect(filed?.method).toBe("POST");
    expect(filed?.body).toEqual({ object_kind: "dataset", object_id: "ds_1", role: "viewer", reason: "Retention study", project: "Persistence 2027", days: 14 });
  });

  it("shows the server's words when the request is refused", async () => {
    serve({ status: 400, body: { error: "This dataset is published on its own, and no app holds its rows, so there is no access to grant. Ask its steward or a Platform Admin to share it" } });
    await open();
    fireEvent.change(screen.getByLabelText(/Reason/), { target: { value: "r" } });
    fireEvent.change(screen.getByLabelText(/Project/), { target: { value: "p" } });
    fireEvent.change(screen.getByLabelText(/Days of access/), { target: { value: "7" } });
    fireEvent.click(screen.getByRole("button", { name: "Send request" }));
    const alert = await screen.findByRole("alert");
    await waitFor(() => expect(alert.textContent).toContain("published on its own"));
  });

  it("says in words where each state of a request is, and a refusal carries its reason", () => {
    expect(whereIsIt({ ...waiting, status: "Rejected", decision_note: "Ask for one course" })).toBe("Refused. Ask for one course");
    expect(whereIsIt({ ...waiting, status: "Approved", ends_at: "2026-12-10T00:00:00Z" })).toBe("Approved. You have access until 2026-12-10.");
    expect(whereIsIt(waiting)).toBe("Waiting for Department chair: chair@x.edu.");
    expect(whereIsIt({ ...waiting, status: "Waiting for person" })).toBe("It waits for you to confirm it.");
  });
});
