import React from "react";
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import "@testing-library/jest-dom";
import { AppBuilder } from "../AppBuilder";
import { PublishedAppView } from "../PublishedAppView";
import { RegisteredApp, AppTable } from "../types";
import {
  applyCompoundFilter,
  applyMultiSort,
  computeFieldValue,
  evaluateClientFormula,
  getFieldTypeIcon,
  groupRecordsByField,
} from "../computedFields";

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

describe("Milestone 2: Multi-View Engine & Data Shaping Usability", () => {
  const records = [
    { id: "R1", title: "Study A", department: "Physics", status: "Approved", budget: 800000 },
    { id: "R2", title: "Study B", department: "Bioengineering", status: "Under Review", budget: 450000 },
    { id: "R3", title: "Study C", department: "Physics", status: "Funded", budget: 600000 },
  ];

  it("filters records with compound AND/OR logic", () => {
    // AND
    const andFiltered = applyCompoundFilter(records, {
      conjunction: "AND",
      clauses: [
        { id: "c1", field_name: "department", operator: "equals", value: "Physics" },
        { id: "c2", field_name: "budget", operator: "greater_than", value: "700000" },
      ],
    });
    expect(andFiltered.length).toBe(1);
    expect(andFiltered[0].id).toBe("R1");

    // OR
    const orFiltered = applyCompoundFilter(records, {
      conjunction: "OR",
      clauses: [
        { id: "c1", field_name: "status", operator: "equals", value: "Under Review" },
        { id: "c2", field_name: "budget", operator: "greater_than", value: "750000" },
      ],
    });
    expect(orFiltered.length).toBe(2);
  });

  it("sorts records across multiple columns", () => {
    const sorted = applyMultiSort(records, [
      { id: "s1", field_name: "department", direction: "asc" },
      { id: "s2", field_name: "budget", direction: "desc" },
    ]);
    expect(sorted[0].id).toBe("R2"); // Bioengineering
    expect(sorted[1].id).toBe("R1"); // Physics 800k
    expect(sorted[2].id).toBe("R3"); // Physics 600k
  });

  it("groups records by field and calculates group subtotals", () => {
    const groups = groupRecordsByField(records, "department");
    expect(groups.length).toBe(2);

    const physicsGroup = groups.find((g) => g.groupValue === "Physics");
    expect(physicsGroup).toBeDefined();
    expect(physicsGroup?.records.length).toBe(2);
    expect(physicsGroup?.totalBudget).toBe(1400000);
  });

  it("switches between Grid, Kanban, and Gallery views interactively in AppBuilder", () => {
    render(
      <AppBuilder
        app={mockApp}
        onBack={vi.fn()}
        onOpenPublishedApp={vi.fn()}
      />
    );

    // Switch to Data tab
    fireEvent.click(screen.getByTestId("tab-btn-data"));

    // Verify view tabs exist
    expect(screen.getByTestId("view-tab-view-proposals-grid")).toBeInTheDocument();
    expect(screen.getByTestId("view-tab-view-proposals-kanban")).toBeInTheDocument();
    expect(screen.getByTestId("view-tab-view-proposals-calendar")).toBeInTheDocument();
    expect(screen.getByTestId("view-tab-view-proposals-gallery")).toBeInTheDocument();

    // Default view is Grid
    expect(screen.getByTestId("view-grid-table")).toBeInTheDocument();

    // Switch to Kanban View
    fireEvent.click(screen.getByTestId("view-tab-view-proposals-kanban"));
    expect(screen.getByTestId("view-kanban-board")).toBeInTheDocument();
    expect(screen.getByTestId("kanban-column-under-review")).toBeInTheDocument();
    expect(screen.getByTestId("kanban-column-approved")).toBeInTheDocument();
    expect(screen.getByTestId("kanban-column-funded")).toBeInTheDocument();

    // Switch to Gallery View
    fireEvent.click(screen.getByTestId("view-tab-view-proposals-gallery"));
    expect(screen.getByTestId("view-gallery-grid")).toBeInTheDocument();
    expect(screen.getByTestId("gallery-card-APP-001")).toBeInTheDocument();

    // Switch back to Grid View
    fireEvent.click(screen.getByTestId("view-tab-view-proposals-grid"));
    expect(screen.getByTestId("view-grid-table")).toBeInTheDocument();
  });

  it("opens filter popover, toggles conjunction, and applies row density presets", () => {
    render(
      <AppBuilder
        app={mockApp}
        onBack={vi.fn()}
        onOpenPublishedApp={vi.fn()}
      />
    );

    // Switch to Data tab
    fireEvent.click(screen.getByTestId("tab-btn-data"));

    // Open Filter Popover
    const filterBtn = screen.getByTestId("toolbar-filter-btn");
    fireEvent.click(filterBtn);
    expect(screen.getByTestId("filter-popover")).toBeInTheDocument();
    expect(screen.getByText("Compound Filter Conditions")).toBeInTheDocument();

    // Add Condition
    fireEvent.click(screen.getByText("+ Add Condition"));
    expect(screen.getByPlaceholderText("Value...")).toBeInTheDocument();

    // Open Sort Popover
    const sortBtn = screen.getByTestId("toolbar-sort-btn");
    fireEvent.click(sortBtn);
    expect(screen.getByTestId("sort-popover")).toBeInTheDocument();
    expect(screen.getByText("Multi-Column Sorting")).toBeInTheDocument();

    // Test row density buttons
    const densityCompact = screen.getByTestId("density-btn-compact");
    const densityTall = screen.getByTestId("density-btn-tall");
    fireEvent.click(densityCompact);
    fireEvent.click(densityTall);

    // Test group by selector
    const groupSelect = screen.getByTestId("toolbar-group-select");
    fireEvent.change(groupSelect, { target: { value: "department" } });
    expect(screen.getAllByText(/Subtotal:/i).length).toBeGreaterThan(0);
  });
});


