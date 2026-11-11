import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { OutboundAccess } from "../admin/OutboundAccess";
import { Webhooks } from "../admin/Webhooks";
import { Overview } from "../admin/Overview";
import { apiClient, type AdminOverviewData, type EgressEntry, type WebhookDelivery, type WebhookRow } from "../api";

const listed: EgressEntry[] = [
  { host: "registrar.example.edu", ports: [443] },
  { cidr: "10.20.0.0/16", ports: [8443, 9443] },
];

// Integrations phase 5, test 5: the allowlist editor asks for a reason, and `Test` shows the explanation.
describe("the Outbound access panel", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("lists each entry as a host or a network, with its ports", async () => {
    vi.spyOn(apiClient, "getEgress").mockResolvedValue(listed);
    render(<OutboundAccess />);
    expect(await screen.findByText("registrar.example.edu")).toBeInTheDocument();
    expect(screen.getByText("10.20.0.0/16")).toBeInTheDocument();
    expect(screen.getByText("8443, 9443")).toBeInTheDocument();
  });

  it("will not save an added entry without a reason, then saves the whole list with it", async () => {
    vi.spyOn(apiClient, "getEgress").mockResolvedValue(listed);
    const save = vi.spyOn(apiClient, "saveEgress").mockResolvedValue(undefined);
    render(<OutboundAccess />);
    fireEvent.click(await screen.findByRole("button", { name: "Add entry" }));
    fireEvent.change(screen.getByLabelText("Host"), { target: { value: "feed.example.edu" } });
    fireEvent.change(screen.getByLabelText("Ports"), { target: { value: "443, 8443" } });
    const submit = screen.getByRole("button", { name: "Save entry" });
    expect(submit).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Reason"), { target: { value: "The registrar feed moves here" } });
    expect(submit).toBeEnabled();
    fireEvent.click(submit);
    await waitFor(() =>
      expect(save).toHaveBeenCalledWith([...listed, { host: "feed.example.edu", ports: [443, 8443] }], "The registrar feed moves here"),
    );
  });

  it("edits an entry's ports, and removes one, each with a reason", async () => {
    vi.spyOn(apiClient, "getEgress").mockResolvedValue(listed);
    const save = vi.spyOn(apiClient, "saveEgress").mockResolvedValue(undefined);
    render(<OutboundAccess />);
    fireEvent.click(await screen.findByRole("button", { name: "Edit registrar.example.edu" }));
    fireEvent.change(screen.getByLabelText("Ports"), { target: { value: "443, 444" } });
    fireEvent.change(screen.getByLabelText("Reason"), { target: { value: "Second port" } });
    fireEvent.click(screen.getByRole("button", { name: "Save entry" }));
    await waitFor(() => expect(save).toHaveBeenCalledWith([{ host: "registrar.example.edu", ports: [443, 444] }, listed[1]], "Second port"));

    fireEvent.click(await screen.findByRole("button", { name: "Remove 10.20.0.0/16" }));
    fireEvent.change(screen.getByRole("textbox", { name: /reason/i }), { target: { value: "Network retired" } });
    fireEvent.click(screen.getByRole("button", { name: "Remove 10.20.0.0/16" }));
    await waitFor(() => expect(save).toHaveBeenLastCalledWith([listed[0]], "Network retired"));
  });

  it("shows the server's words when Test is pressed on a private address", async () => {
    vi.spyOn(apiClient, "getEgress").mockResolvedValue(listed);
    const test = vi.spyOn(apiClient, "testEgress").mockResolvedValue({
      host: "10.0.0.5",
      port: 443,
      allowed: false,
      address: null,
      explanation: "Blocked. 10.0.0.5 resolves to a private address, which no cidr entry allows.",
    });
    render(<OutboundAccess />);
    fireEvent.change(await screen.findByLabelText("Host to test"), { target: { value: "10.0.0.5" } });
    fireEvent.click(screen.getByRole("button", { name: "Test" }));
    await waitFor(() => expect(test).toHaveBeenCalledWith("10.0.0.5", 443));
    expect(await screen.findByText(/resolves to a private address/)).toBeInTheDocument();
  });
});

const row: WebhookRow = {
  app: "housing",
  app_title: "Housing",
  id: "notify-housing",
  name: "Notify housing",
  host: "hooks.example.org",
  events: ["RecordCreated", "RecordUpdated"],
  state: "Failing",
  consecutive_failures: 3,
  last_success: "2026-11-10T09:30:00Z",
};

const log: WebhookDelivery[] = [
  { id: "d1", event: "record.created", record: "r1", state: "queued", attempts: 2, last_status: 500, last_error: "The receiver answered 500", created_at: "2026-11-11T08:00:00Z", delivered_at: null },
  { id: "d2", event: "record.updated", record: "r2", state: "dead", attempts: 8, last_status: null, last_error: "Turned off by an administrator: Maintenance", created_at: "2026-11-10T08:00:00Z", delivered_at: null },
];

describe("the Webhooks panel", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("lists every webhook with its app, host, events, state, last success and failures", async () => {
    vi.spyOn(apiClient, "listWebhooks").mockResolvedValue([row]);
    render(<Webhooks />);
    const item = (await screen.findByText("Notify housing")).closest("li") as HTMLElement;
    expect(within(item).getByText("Housing")).toBeInTheDocument();
    expect(within(item).getByText("hooks.example.org")).toBeInTheDocument();
    expect(within(item).getByText("RecordCreated, RecordUpdated")).toBeInTheDocument();
    expect(within(item).getByText("Failing")).toBeInTheDocument();
    expect(within(item).getByText(/3 failures in a row/)).toBeInTheDocument();
    expect(within(item).getByText(/Last success/)).toBeInTheDocument();
  });

  it("opens the recent deliveries with their status codes and errors", async () => {
    vi.spyOn(apiClient, "listWebhooks").mockResolvedValue([row]);
    const deliveries = vi.spyOn(apiClient, "webhookDeliveries").mockResolvedValue(log);
    render(<Webhooks />);
    fireEvent.click(await screen.findByRole("button", { name: "Deliveries of Notify housing" }));
    await waitFor(() => expect(deliveries).toHaveBeenCalledWith("housing", "notify-housing"));
    expect(await screen.findByText("The receiver answered 500")).toBeInTheDocument();
    expect(screen.getByText("500")).toBeInTheDocument();
    expect(screen.getByText(/Turned off by an administrator: Maintenance/)).toBeInTheDocument();
    expect(screen.getByText("dead")).toBeInTheDocument();
  });

  it("disables a webhook only with a reason", async () => {
    vi.spyOn(apiClient, "listWebhooks").mockResolvedValue([row]);
    const set = vi.spyOn(apiClient, "setWebhookEnabled").mockResolvedValue(undefined);
    render(<Webhooks />);
    fireEvent.click(await screen.findByRole("button", { name: "Disable Notify housing" }));
    const confirm = screen.getByRole("button", { name: "Disable Notify housing" });
    expect(confirm).toBeDisabled();
    fireEvent.change(screen.getByRole("textbox", { name: /reason/i }), { target: { value: "The receiver is being replaced" } });
    fireEvent.click(confirm);
    await waitFor(() => expect(set).toHaveBeenCalledWith("housing", "notify-housing", false, "The receiver is being replaced"));
  });

  it("offers Enable, not Disable, for a webhook that is off", async () => {
    vi.spyOn(apiClient, "listWebhooks").mockResolvedValue([{ ...row, state: "Disabled" }]);
    render(<Webhooks />);
    expect(await screen.findByRole("button", { name: "Enable Notify housing" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Disable Notify housing" })).not.toBeInTheDocument();
  });
});

const overview = (outbound: AdminOverviewData["outbound"]): AdminOverviewData => ({
  server: { version: "0.1.0", applied_migrations: [], database_worker_count: 4 },
  people: { active: 3, on_hold: 0, inactive: 0, platform_admins: 1, active_tokens: {} },
  organization: { unit_count: 1 },
  workspaces: { workspace_count: 1, app_count: 1 },
  processes: { waiting_instance_count: 0 },
  ledger: { entry_count: 1, head_hash: "abc" },
  outbound,
});

describe("the Overview's outbound card", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("counts failing and disabled webhooks and dead deliveries in the last day", async () => {
    vi.spyOn(apiClient, "getAdminOverview").mockResolvedValue(overview({ webhooks_failing: 2, webhooks_disabled: 1, deliveries_dead_last_day: 7 }));
    render(<Overview />);
    const card = await screen.findByTestId("admin-overview-outbound-card");
    expect(within(card).getByTestId("overview-webhooks-failing")).toHaveTextContent("2");
    expect(within(card).getByTestId("overview-webhooks-disabled")).toHaveTextContent("1");
    expect(within(card).getByTestId("overview-deliveries-dead")).toHaveTextContent("7");
  });
});
