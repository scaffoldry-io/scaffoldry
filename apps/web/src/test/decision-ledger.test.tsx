import "@testing-library/jest-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { DecisionLedgerView } from "../DecisionLedgerView";
import { apiClient } from "../api";

const entry = (sequence: number) => ({
  sequence,
  entry_hash: `hash${sequence}`.padEnd(64, "0"),
  previous_hash: `hash${sequence - 1}`.padEnd(64, "0"),
  timestamp_iso: "2026-11-18T10:00:00Z",
  principal: "setup@scaffoldry.internal",
  organization_code: "INST",
  app_slug: null,
  decision_type: "PolicyRevision",
  oscal_control_id: "AC-03",
  rationale: `Entry ${sequence}`,
});

const page = (over = {}) => ({
  chain_valid: null,
  total_entries: 16,
  head_hash: "head".padEnd(64, "f"),
  next_cursor: null,
  entries: [entry(16), entry(15)],
  ...over,
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("the decision ledger", () => {
  it("reads the ledger from the server when it opens and counts every entry the server holds", async () => {
    const list = vi.spyOn(apiClient, "getGovernanceLedger").mockResolvedValue(page() as never);
    render(<DecisionLedgerView />);
    expect(await screen.findByText("16 Blocks Recorded")).toBeInTheDocument();
    expect(list).toHaveBeenCalledTimes(1);
    expect(screen.getByText(/Showing 2 of 16 entries/)).toBeInTheDocument();
  });

  it("does not claim the chain is valid before anyone checks it", async () => {
    vi.spyOn(apiClient, "getGovernanceLedger").mockResolvedValue(page() as never);
    render(<DecisionLedgerView />);
    await screen.findByText("16 Blocks Recorded");
    expect(screen.queryByText(/Zero Tampering/)).not.toBeInTheDocument();
    expect(screen.getByText("Not checked yet")).toBeInTheDocument();
  });

  it("asks the server to verify the chain and shows its answer", async () => {
    vi.spyOn(apiClient, "getGovernanceLedger").mockResolvedValue(page() as never);
    const verify = vi
      .spyOn(apiClient, "verifyGovernanceLedger")
      .mockResolvedValue({ verified: false, total_entries: 16, genesis_previous_hash: "0".repeat(64), head_hash: "x", verification_status: "Entry 7 does not follow entry 6" });
    render(<DecisionLedgerView />);
    await screen.findByText("16 Blocks Recorded");
    fireEvent.click(screen.getByRole("button", { name: /Verify Chain Integrity/ }));
    await waitFor(() => expect(verify).toHaveBeenCalledTimes(1));
    expect(await screen.findByText("Verification Failed")).toBeInTheDocument();
    expect(screen.getByText(/Entry 7 does not follow entry 6/)).toBeInTheDocument();
  });

  it("shows a valid chain only when the server says so", async () => {
    vi.spyOn(apiClient, "getGovernanceLedger").mockResolvedValue(page() as never);
    vi.spyOn(apiClient, "verifyGovernanceLedger").mockResolvedValue({ verified: true, total_entries: 16, genesis_previous_hash: "0".repeat(64), head_hash: "x", verification_status: "valid" });
    render(<DecisionLedgerView />);
    await screen.findByText("16 Blocks Recorded");
    fireEvent.click(screen.getByRole("button", { name: /Verify Chain Integrity/ }));
    expect(await screen.findByText("Zero Tampering (Valid)")).toBeInTheDocument();
  });

  it("says so when the ledger cannot be read", async () => {
    vi.spyOn(apiClient, "getGovernanceLedger").mockRejectedValue(new Error("The server cannot read its database"));
    render(<DecisionLedgerView />);
    expect(await screen.findByText(/cannot read its database/)).toBeInTheDocument();
  });
});
