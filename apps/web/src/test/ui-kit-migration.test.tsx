import "@testing-library/jest-dom";
import React, { useState } from "react";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { WorkspaceSettingsModal } from "../WorkspaceSettingsModal";
import { CoBuilderStudioModal } from "../CoBuilderStudioModal";
import { AddViewModal } from "../AddViewModal";
import { CreateTableModal } from "../CreateTableModal";
import { LinkedFieldModal } from "../LinkedFieldModal";
import { CsvImportModal } from "../CsvImportModal";
import { Trash } from "../Trash";
import { DelegationBanner } from "../DelegationPanel";
import { apiClient } from "../api";
import type { AppTable, Persona, Workspace } from "../types";

const persona: Persona = {
  eppn: "owner@state.edu",
  name: "Owner One",
  affiliation: "faculty",
  department: "biology",
  roleTitle: "Professor",
  isAdmin: false,
};
const workspace: Workspace = {
  id: "ws-1",
  name: "Biology Lab",
  code: "BIO",
  department: "biology",
  description: "Lab",
  icon: "B",
  lead: persona.eppn,
  visibility: "restricted",
  data_classification: "Level 4 Restricted",
  created_at: new Date().toISOString(),
  collaborators: [
    { id: "c1", eppn: persona.eppn, name: "Owner One", role: "owner", department: "biology", scoped_affiliation: "faculty", added_at: "" },
  ],
};
const table: AppTable = {
  id: "t1",
  name: "People",
  slug: "people",
  fields: [{ name: "name", label: "Name", type: "text" }],
  records: [],
};
const otherTable: AppTable = { ...table, id: "t2", name: "Places", slug: "places" };
const studioApp: any = {
  slug: "app-1",
  title: "Studio App",
  orgCode: "X",
  department: "biology",
  customDomain: "",
  verified: false,
  hermCapability: "",
  cedsDomain: "",
  status: "Draft",
  updatedAt: "",
  recordsCount: 0,
  workspaceId: "ws-1",
  collaborators: [],
  manifest: { views: [{ fields: [] }] },
};

/** An opener button, and the modal it opens. Open it by clicking the button. */
function Harness({ children }: { children: (open: boolean, close: () => void) => React.ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <div>
      <button onClick={() => setOpen(true)}>Open it</button>
      {children(open, () => setOpen(false))}
    </div>
  );
}

const noop = () => {};
const cases: [string, (open: boolean, close: () => void) => React.ReactNode][] = [
  [
    "WorkspaceSettingsModal",
    (open, close) => (
      <WorkspaceSettingsModal workspace={workspace} activePersona={persona} allPersonas={[persona]} isOpen={open} onClose={close} onSave={noop} ledgerEntries={[]} />
    ),
  ],
  [
    "CoBuilderStudioModal",
    (open, close) => (
      <CoBuilderStudioModal
        isOpen={open}
        onClose={close}
        activeStudioApp={studioApp}
        setActiveStudioApp={noop}
        studioTab="schema"
        setStudioTab={noop}
        automations={{}}
        handleSaveRule={noop}
        handleDeleteRule={noop}
        handleToggleRule={noop}
        handleAddLinkedDatasetField={noop}
        handleAddFieldToStudioApp={noop}
        handleToggleFieldFerpa={noop}
        handleAddCollaborator={noop}
        handlePublishApp={noop}
        showToast={noop}
        personas={[persona]}
        setApps={noop}
      />
    ),
  ],
  [
    "AddViewModal",
    (open, close) => (
      <AddViewModal isOpen={open} onClose={close} newViewTitle="" setNewViewTitle={noop} newViewType="Grid" setNewViewType={noop} onCreateNewView={noop} />
    ),
  ],
  [
    "CreateTableModal",
    (open, close) => (
      <CreateTableModal isOpen={open} onClose={close} newTableName="" setNewTableName={noop} newTableIcon="" setNewTableIcon={noop} onAddNewTable={noop} />
    ),
  ],
  [
    "LinkedFieldModal",
    (open, close) => <LinkedFieldModal isOpen={open} onClose={close} tables={[table, otherTable]} activeTableId="t1" onSave={noop} />,
  ],
  ["CsvImportModal", (open, close) => <CsvImportModal isOpen={open} onClose={close} activeTable={table} />],
];

describe("Phase 3: hand-written modals sit on the kit", () => {
  beforeEach(() => {
    vi.spyOn(apiClient, "listDatasets").mockResolvedValue([]);
    vi.spyOn(apiClient, "getWorkspaceGuards").mockResolvedValue({ current: null, history: [] } as any);
    vi.spyOn(apiClient, "listOrganizations").mockResolvedValue([]);
  });
  afterEach(() => vi.restoreAllMocks());

  it.each(cases)("%s opens with focus inside, and Escape closes it and returns focus", async (_name, make) => {
    render(<Harness>{make}</Harness>);
    const opener = screen.getByRole("button", { name: "Open it" });
    opener.focus();
    fireEvent.click(opener);
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(dialog.getAttribute("aria-labelledby")).toBeTruthy();
    expect(dialog.contains(document.activeElement)).toBe(true);
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(opener).toHaveFocus();
  });
});

describe("Phase 3: no browser dialogs", () => {
  beforeEach(() => {
    vi.spyOn(apiClient, "getWorkspaceGuards").mockResolvedValue({ current: null, history: [] } as any);
    vi.spyOn(apiClient, "listOrganizations").mockResolvedValue([]);
  });
  afterEach(() => vi.restoreAllMocks());

  it("says in a Toast that the only owner cannot be removed, and calls no window.alert", async () => {
    const alertSpy = vi.spyOn(window, "alert").mockImplementation(() => {});
    render(
      <WorkspaceSettingsModal workspace={workspace} activePersona={persona} allPersonas={[persona]} isOpen onClose={noop} onSave={noop} ledgerEntries={[]} />,
    );
    fireEvent.click(screen.getByTestId("ws-settings-tab-members"));
    fireEvent.click(await screen.findByTestId(`ws-member-remove-btn-${persona.eppn}`));
    expect(await screen.findByRole("alert")).toHaveTextContent("Cannot remove the only workspace owner");
    expect(alertSpy).not.toHaveBeenCalled();
  });

  it("says in a Toast that the person is already a member, and calls no window.alert", async () => {
    const alertSpy = vi.spyOn(window, "alert").mockImplementation(() => {});
    render(
      <WorkspaceSettingsModal workspace={workspace} activePersona={persona} allPersonas={[persona]} isOpen onClose={noop} onSave={noop} ledgerEntries={[]} />,
    );
    fireEvent.click(screen.getByTestId("ws-settings-tab-members"));
    fireEvent.click(await screen.findByTestId("ws-add-member-btn"));
    expect(await screen.findByRole("alert")).toHaveTextContent("already a member");
    expect(alertSpy).not.toHaveBeenCalled();
  });
});

describe("Phase 3: failures are shown, not swallowed", () => {
  afterEach(() => vi.restoreAllMocks());

  it("Trash shows its ErrorState, and Retry shows it again without an unhandled rejection", async () => {
    const unhandled = vi.fn();
    process.on("unhandledRejection", unhandled);
    const api = await import("../api");
    const spy = vi.spyOn(api, "listTrash").mockRejectedValue(new Error("Trash is down"));
    render(<Trash appSlug="demo" />);
    expect(await screen.findByText(/Trash is down/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /retry/i }));
    await waitFor(() => expect(spy).toHaveBeenCalledTimes(2));
    await new Promise((r) => setTimeout(r, 20));
    process.off("unhandledRejection", unhandled);
    expect(screen.getByText(/Trash is down/)).toBeInTheDocument();
    expect(unhandled).not.toHaveBeenCalled();
  });

  it("DelegationBanner says it could not check delegations", async () => {
    vi.spyOn(apiClient, "listMyDelegations").mockRejectedValue(new Error("Server unreachable"));
    render(<DelegationBanner />);
    expect(await screen.findByRole("alert")).toHaveTextContent(/delegations/i);
  });
});
