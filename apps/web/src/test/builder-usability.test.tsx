import React from "react";
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import "@testing-library/jest-dom";
import { AppBuilder } from "../AppBuilder";
import { PublishedAppView } from "../PublishedAppView";
import { RegisteredApp, AppTable } from "../types";
import { computeFieldValue, evaluateClientFormula, getFieldTypeIcon } from "../computedFields";

const mockApp: RegisteredApp = {
  slug: "physics-grants",
  title: "Physics Department Research Grants",
  orgCode: "DIV-SCIENCES",
  department: "Physics & Astronomy",
  customDomain: "grants.physics.science.state.edu",
  verified: true,
  hermCapability: "2.1.0 (Academic Operations)",
  cedsDomain: "PostsecondaryStudent",
  status: "Published",
  updatedAt: "2026-10-04",
  recordsCount: 4,
  workspaceId: "ws-physics",
  collaborators: [
    {
      eppn: "prof.curie@science.state.edu",
      name: "Dr. Marie Curie",
      role: "owner",
      department: "Physics & Astronomy",
    },
  ],
  manifest: {
    slug: "physics-grants",
    title: "Physics Department Research Grants",
    description: "Governed collaborative grant proposals and faculty review workflow.",
    organization_code: "DIV-SCIENCES",
    department: "Physics & Astronomy",
    custom_domain_verified: true,
    views: [
      {
        id: "v1",
        title: "Default View",
        view_type: "Table",
        fields: [
          { name: "id", label: "ID", field_type: "Text", required: true, ferpa_sensitive: false },
          { name: "title", label: "Title", field_type: "Text", required: true, ferpa_sensitive: false },
          { name: "budget", label: "Budget", field_type: "Number", required: true, ferpa_sensitive: false },
        ],
      },
    ],
    ceds_mappings: {},
  },
};

describe("AppBuilder Usability & Interactive Component Canvas", () => {
  it("renders full-page builder with top bar and switches between tabs", () => {
    const handleBack = vi.fn();
    const handleOpenPublished = vi.fn();

    render(
      <AppBuilder
        app={mockApp}
        onBack={handleBack}
        onOpenPublishedApp={handleOpenPublished}
      />
    );

    // Verify top bar details
    expect(screen.getByText("Physics Department Research Grants")).toBeInTheDocument();
    expect(screen.getByText("/builder/physics-grants")).toBeInTheDocument();
    expect(screen.getByText(/3 Tables · 2 Relations/i)).toBeInTheDocument();

    // Verify tabs
    const tabPages = screen.getByTestId("tab-btn-pages");
    const tabData = screen.getByTestId("tab-btn-data");
    const tabAutomations = screen.getByTestId("tab-btn-automations");
    const tabSettings = screen.getByTestId("tab-btn-settings");

    expect(tabPages).toBeInTheDocument();
    expect(tabData).toBeInTheDocument();
    expect(tabAutomations).toBeInTheDocument();
    expect(tabSettings).toBeInTheDocument();

    // Switch to Settings tab
    fireEvent.click(tabSettings);
    expect(screen.getByText("Multi-Table Storage & Vanity Routing")).toBeInTheDocument();
    expect(screen.getByText("Relational Lattice Topology")).toBeInTheDocument();

    // Switch back to Pages tab
    fireEvent.click(tabPages);
    expect(screen.getByText("Application Pages")).toBeInTheDocument();
  });

  it("selects a component on canvas and updates properties via flyout inspector", () => {
    render(
      <AppBuilder
        app={mockApp}
        onBack={vi.fn()}
        onOpenPublishedApp={vi.fn()}
      />
    );

    // Click on Active Proposals stat component
    const statComp = screen.getByTestId("canvas-component-comp-stat-1");
    expect(statComp).toBeInTheDocument();
    fireEvent.click(statComp);

    // Verify Flyout Inspector opens
    const flyout = screen.getByTestId("component-inspector-flyout");
    expect(flyout).toBeInTheDocument();
    expect(screen.getByText("Component Configuration")).toBeInTheDocument();

    // Edit component title in flyout
    const titleInput = screen.getByDisplayValue("Active Proposals");
    fireEvent.change(titleInput, { target: { value: "Approved Faculty Grants" } });

    // Assert the component title on the visual canvas updated reactively
    expect(screen.getByText("Approved Faculty Grants")).toBeInTheDocument();
  });

  it("supports multiple tables per app and foreign key relational lookups in Data tab", () => {
    render(
      <AppBuilder
        app={mockApp}
        onBack={vi.fn()}
        onOpenPublishedApp={vi.fn()}
      />
    );

    // Switch to Data & Schema tab
    const tabData = screen.getByTestId("tab-btn-data");
    fireEvent.click(tabData);

    // Verify multi-table switcher buttons exist
    expect(screen.getByTestId("table-tab-proposals")).toBeInTheDocument();
    expect(screen.getByTestId("table-tab-investigators")).toBeInTheDocument();
    expect(screen.getByTestId("table-tab-allocations")).toBeInTheDocument();

    // Verify relational badge in Proposals table
    expect(screen.getAllByText("Dr. Marie Curie").length).toBeGreaterThan(0);

    // Switch to Principal Investigators table
    fireEvent.click(screen.getByTestId("table-tab-investigators"));
    expect(screen.getAllByText("Principal Investigators").length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText("Tenured and tenure-track faculty research leaders")).toBeInTheDocument();
    expect(screen.getByText("Dr. Alan Turing")).toBeInTheDocument();

    // Switch to Budget Allocations table
    fireEvent.click(screen.getByTestId("table-tab-allocations"));
    expect(screen.getAllByText("Budget Allocations").length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText("Grant expenditure allocations and quarterly disbursement tranches")).toBeInTheDocument();
    expect(screen.getByText("ALC-101")).toBeInTheDocument();
  });
});

describe("PublishedAppView Standalone Runtime Usability", () => {
  it("renders published end-user application shell with navigation and records", () => {
    const handleOpenBuilder = vi.fn();
    const handleBack = vi.fn();

    render(
      <PublishedAppView
        app={mockApp}
        onOpenBuilder={handleOpenBuilder}
        onBackToDesk={handleBack}
      />
    );

    // Assert published header
    expect(screen.getByTestId("published-app-runtime")).toBeInTheDocument();
    expect(screen.getByText("Physics Department Research Grants")).toBeInTheDocument();
    expect(screen.getByText("Live")).toBeInTheDocument();

    // Assert records displayed
    expect(screen.getByText("Quantum Optomechanics Qubit Study")).toBeInTheDocument();
    expect(screen.getByText("Neural Stem Cell Regeneration Assay")).toBeInTheDocument();

    // Switch to Approval Kanban page
    const kanbanNav = screen.getByTestId("page-nav-stages");
    fireEvent.click(kanbanNav);
    expect(screen.getByText("Review Stage Progression")).toBeInTheDocument();
    expect(screen.getAllByText("Under Review").length).toBeGreaterThan(0);
  });

  it("submits a new record through the intake modal and commits to local state", () => {
    render(
      <PublishedAppView
        app={mockApp}
      />
    );

    // Click + New Proposal button
    const openBtn = screen.getByText("+ New Proposal");
    fireEvent.click(openBtn);

    // Verify modal is open
    expect(screen.getByText("Submit New Research Proposal")).toBeInTheDocument();

    // Fill form
    const titleInput = screen.getByPlaceholderText("e.g. Sub-Kelvin Topological Insulator Measurement");
    fireEvent.change(titleInput, { target: { value: "Topological Superconductivity in Dirac Semimetals" } });

    // Submit form
    const form = screen.getByTestId("proposal-intake-form");
    fireEvent.submit(form);

    // Assert new record appears in the published view
    expect(screen.getByText("Topological Superconductivity in Dirac Semimetals")).toBeInTheDocument();
    expect(screen.getByText(/committed to sovereign ledger/i)).toBeInTheDocument();
  });

  it("renders relational lookup badges and links foreign key in intake modal", () => {
    render(
      <PublishedAppView
        app={mockApp}
      />
    );

    // Verify relational badge rendered for records
    expect(screen.getByTestId("rel-badge-APP-101")).toHaveTextContent("Dr. Marie Curie");
    expect(screen.getByTestId("rel-badge-APP-103")).toHaveTextContent("Dr. Alan Turing");

    // Open proposal modal
    fireEvent.click(screen.getByText("+ New Proposal"));

    // Select Principal Investigator
    const piSelect = screen.getByTestId("proposal-pi-select");
    expect(piSelect).toBeInTheDocument();
    fireEvent.change(piSelect, { target: { value: "INV-02" } }); // Alan Turing

    const titleInput = screen.getByPlaceholderText("e.g. Sub-Kelvin Topological Insulator Measurement");
    fireEvent.change(titleInput, { target: { value: "Neural Network Morphisms" } });

    // Submit form
    fireEvent.submit(screen.getByTestId("proposal-intake-form"));

    // Verify new record has Dr. Alan Turing badge
    expect(screen.getByText("Neural Network Morphisms")).toBeInTheDocument();
    expect(screen.getByTestId("rel-badge-APP-105")).toHaveTextContent("Dr. Alan Turing");
  });
});

describe("Milestone 1: Rich Field Types & Computed Field Engine Usability", () => {
  it("evaluates in-memory formulas with token substitution and arithmetic", () => {
    const record = {
      budget: 500000,
      spent: 120000,
      first_name: "Ada",
      last_name: "Lovelace",
    };

    expect(evaluateClientFormula("{budget}", record)).toBe(500000);
    expect(evaluateClientFormula("{budget} * 0.20", record)).toBe(100000);
    expect(evaluateClientFormula("{budget} / 10", record)).toBe(50000);
    expect(evaluateClientFormula("{budget} - {spent}", record)).toBe(380000);
    expect(evaluateClientFormula('{first_name} + " " + {last_name}', record)).toBe("Ada Lovelace");
  });

  it("computes relational lookups, counts, and rollups across tables reactively", () => {
    const sampleTables: AppTable[] = [
      {
        id: "tbl-main",
        name: "Proposals",
        slug: "proposals",
        primary_field: "id",
        fields: [],
        records: [{ id: "REC-1", lead_id: "FAC-1" }],
      },
      {
        id: "tbl-faculty",
        name: "Faculty",
        slug: "faculty",
        primary_field: "id",
        fields: [],
        records: [{ id: "FAC-1", email: "faculty@state.edu" }],
      },
      {
        id: "tbl-items",
        name: "Items",
        slug: "items",
        primary_field: "id",
        fields: [],
        records: [
          { id: "ITM-1", parent_id: "REC-1", amount: 100 },
          { id: "ITM-2", parent_id: "REC-1", amount: 200 },
          { id: "ITM-3", parent_id: "REC-1", amount: 300 },
        ],
      },
    ];

    // Lookup
    const lookupField = {
      name: "lead_email",
      label: "Lead Email",
      field_type: "Lookup" as const,
      required: false,
      ferpa_sensitive: false,
      target_table_id: "tbl-faculty",
      target_display_field: "email",
    };
    expect(computeFieldValue(lookupField, { id: "REC-1", lead_id: "FAC-1" }, sampleTables)).toBe("faculty@state.edu");

    // Count
    const countField = {
      name: "items_count",
      label: "Items Count",
      field_type: "Count" as const,
      required: false,
      ferpa_sensitive: false,
      target_table_id: "tbl-items",
    };
    expect(computeFieldValue(countField, { id: "REC-1" }, sampleTables)).toBe(3);

    // Rollup Sum
    const rollupSumField = {
      name: "total_sum",
      label: "Total Sum",
      field_type: "Rollup" as const,
      required: false,
      ferpa_sensitive: false,
      target_table_id: "tbl-items",
      target_display_field: "amount",
      rollup_function: "sum" as const,
    };
    expect(computeFieldValue(rollupSumField, { id: "REC-1" }, sampleTables)).toBe(600);

    // Rollup Avg
    const rollupAvgField = {
      ...rollupSumField,
      rollup_function: "avg" as const,
    };
    expect(computeFieldValue(rollupAvgField, { id: "REC-1" }, sampleTables)).toBe(200);
  });

  it("renders rich field types and computed cells inside AppBuilder Data Workspace", () => {
    render(
      <AppBuilder
        app={mockApp}
        onBack={vi.fn()}
        onOpenPublishedApp={vi.fn()}
      />
    );

    // Switch to Data tab
    fireEvent.click(screen.getByTestId("tab-btn-data"));

    // Check headers with icons
    expect(screen.getAllByText("Ethics Approved").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Research Tags").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Review Rating").length).toBeGreaterThan(0);
    expect(screen.getAllByText("PI Email").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Total Disbursed").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Disbursement Count").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Indirect Cost (20%)").length).toBeGreaterThan(0);

    // Check MultiSelect pills rendered
    expect(screen.getAllByText("Quantum").length).toBeGreaterThan(0);
    expect(screen.getByText("StemCell")).toBeInTheDocument();

    // Check Lookup email rendered
    expect(screen.getAllByText("curie@science.state.edu").length).toBeGreaterThan(0);

    // Check Rollup sum ($450,000 for APP-001)
    expect(screen.getAllByText("$450,000").length).toBeGreaterThan(0);

    // Check Formula ($90,000 indirect cost for APP-001: 450,000 * 0.20)
    expect(screen.getByText("$90,000")).toBeInTheDocument();
  });
});

