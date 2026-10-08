import React from "react";
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import "@testing-library/jest-dom";
import { AppBuilder } from "../AppBuilder";
import { PublishedAppView } from "../PublishedAppView";
import { StandaloneIntakeForm } from "../StandaloneIntakeForm";
import { AdminDesk } from "../AdminDesk";
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
      rate: 0.2,
      first_name: "Ada",
      last_name: "Lovelace",
      title: "",
    };

    expect(evaluateClientFormula("{budget}", record)).toBe(500000);
    expect(evaluateClientFormula("{budget} * 0.20", record)).toBe(100000);
    expect(evaluateClientFormula("{budget} / 10", record)).toBe(50000);
    expect(evaluateClientFormula("{budget} - {spent}", record)).toBe(380000);
    expect(evaluateClientFormula('{first_name} + " " + {last_name}', record)).toBe("Ada Lovelace");
    expect(evaluateClientFormula("{budget} - {spent} * {rate}", record)).toBe(476000);
    expect(evaluateClientFormula("({budget} - {spent}) * {rate}", record)).toBe(76000);
    expect(evaluateClientFormula("IF({spent} > 100000, {budget} - {spent}, 0)", record)).toBe(380000);
    expect(evaluateClientFormula('IF(ISBLANK({title}), "untitled", {title})', record)).toBe("untitled");
    expect(evaluateClientFormula("ROUND({budget} * {rate}, 0)", record)).toBe(100000);
    expect(evaluateClientFormula("AND({spent} > 0, {budget} > {spent})", record)).toBe(true);
    expect(evaluateClientFormula("{missing}", record)).toBeNull();
    expect(evaluateClientFormula("{budget} / 0", record)).toBeNull();
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

  it("opens record detail drawer on row expand, edits properties, and closes drawer", () => {
    render(
      <AppBuilder
        app={mockApp}
        onBack={vi.fn()}
        onOpenPublishedApp={vi.fn()}
      />
    );

    fireEvent.click(screen.getByTestId("tab-btn-data"));

    // Click expand button on APP-001
    const expandBtn = screen.getByTestId("row-expand-btn-APP-001");
    fireEvent.click(expandBtn);

    // Detail drawer should be visible
    expect(screen.getByTestId("record-detail-drawer")).toBeInTheDocument();
    expect(screen.getByTestId("detail-record-id-badge")).toHaveTextContent("APP-001");

    // Title input
    const titleInput = screen.getByTestId("detail-primary-title-input");
    expect(titleInput).toHaveValue("Quantum Optomechanics Qubit Study");
    fireEvent.change(titleInput, { target: { value: "Updated Quantum Proposal" } });
    expect(titleInput).toHaveValue("Updated Quantum Proposal");

    // Computed field is rendered as calculated
    expect(screen.getByTestId("detail-computed-indirect_cost")).toBeInTheDocument();

    // Close drawer
    fireEvent.click(screen.getByTestId("detail-close-btn"));
    expect(screen.queryByTestId("record-detail-drawer")).not.toBeInTheDocument();
  });

  it("renders reverse relational sub-tables in detail drawer and creates linked record", () => {
    render(
      <AppBuilder
        app={mockApp}
        onBack={vi.fn()}
        onOpenPublishedApp={vi.fn()}
      />
    );

    fireEvent.click(screen.getByTestId("tab-btn-data"));

    // Expand APP-001
    fireEvent.click(screen.getByTestId("row-expand-btn-APP-001"));

    // Reverse relational section
    expect(screen.getByTestId("reverse-relation-section")).toBeInTheDocument();
    expect(screen.getByText("Linked Budget Allocations")).toBeInTheDocument();
    expect(screen.getByText("2 records")).toBeInTheDocument();

    // Add linked record
    const addLinkedBtn = screen.getByTestId("add-linked-tbl-allocations-btn");
    fireEvent.click(addLinkedBtn);
    expect(screen.getByText("3 records")).toBeInTheDocument();
  });

  it("adds rows inline and through toolbar, duplicates and deletes records", () => {
    render(
      <AppBuilder
        app={mockApp}
        onBack={vi.fn()}
        onOpenPublishedApp={vi.fn()}
      />
    );

    fireEvent.click(screen.getByTestId("tab-btn-data"));

    // Inline Add Row
    const inlineAddRowBtn = screen.getByTestId("grid-add-row-btn");
    fireEvent.click(inlineAddRowBtn);
    expect(screen.getAllByText(/New Research Proposals Record/i).length).toBeGreaterThan(0);

    // Toolbar Add Record
    const toolbarAddBtn = screen.getByTestId("toolbar-add-record-btn");
    fireEvent.click(toolbarAddBtn);

    // Duplicate record APP-001
    const duplicateBtn = screen.getByTestId("row-duplicate-btn-APP-001");
    fireEvent.click(duplicateBtn);
    expect(screen.getByText(/Quantum Optomechanics Qubit Study \(Copy\)/i)).toBeInTheDocument();

    // Delete record APP-004
    const deleteBtn = screen.getByTestId("row-delete-btn-APP-004");
    fireEvent.click(deleteBtn);
    expect(screen.queryByText("High-Entropy Alloy Catalyst Synthesis")).not.toBeInTheDocument();
  });

  it("supports bulk selection, batch duplication, and batch deletion", () => {
    render(
      <AppBuilder
        app={mockApp}
        onBack={vi.fn()}
        onOpenPublishedApp={vi.fn()}
      />
    );

    fireEvent.click(screen.getByTestId("tab-btn-data"));

    // Select row APP-001
    const selectCheckbox1 = screen.getByTestId("row-select-checkbox-APP-001");
    fireEvent.click(selectCheckbox1);

    // Floating batch bar should appear
    expect(screen.getByTestId("batch-action-bar")).toBeInTheDocument();
    expect(screen.getByText("1 record selected")).toBeInTheDocument();

    // Select all checkbox
    const selectAllCheckbox = screen.getByTestId("select-all-checkbox");
    fireEvent.click(selectAllCheckbox);
    expect(screen.getByText(/records selected/i)).toBeInTheDocument();

    // Clear selection
    fireEvent.click(screen.getByTestId("batch-clear-btn"));
    expect(screen.queryByTestId("batch-action-bar")).not.toBeInTheDocument();
  });

  it("handles CSV export and CSV import modal with column mapping", () => {
    render(
      <AppBuilder
        app={mockApp}
        onBack={vi.fn()}
        onOpenPublishedApp={vi.fn()}
      />
    );

    fireEvent.click(screen.getByTestId("tab-btn-data"));

    // Open CSV Import modal
    const importCsvBtn = screen.getByTestId("toolbar-import-csv-btn");
    fireEvent.click(importCsvBtn);

    expect(screen.getByTestId("csv-import-modal")).toBeInTheDocument();
    expect(screen.getByText(/Import CSV into Research Proposals/i)).toBeInTheDocument();

    // Enter CSV text
    const textarea = screen.getByTestId("csv-textarea-input");
    fireEvent.change(textarea, {
      target: {
        value: `title,budget,status\n"Laser Interferometry Gravity",950000,Approved`,
      },
    });

    // Verify mapping interface rendered
    expect(screen.getByText(/1 rows found/i)).toBeInTheDocument();
    expect(screen.getByTestId("csv-map-select-title")).toHaveValue("title");
    expect(screen.getByTestId("csv-map-select-budget")).toHaveValue("budget");

    // Execute Import
    const executeBtn = screen.getByTestId("csv-execute-import-btn");
    fireEvent.click(executeBtn);

    // Modal closed and new record present in grid
    expect(screen.queryByTestId("csv-import-modal")).not.toBeInTheDocument();
    expect(screen.getByText("Laser Interferometry Gravity")).toBeInTheDocument();
  });

  describe("Milestone 5: Standalone Public Intake Forms & Full-Screen UI/UX Usability", () => {
    it("renders full-screen StandaloneIntakeForm, switches tables, and validates required fields", () => {
      const mockRichApp: RegisteredApp = {
        ...mockApp,
        manifest: {
          ...mockApp.manifest,
          tables: [
            {
              id: "tbl-proposals",
              name: "Research Proposals",
              slug: "proposals",
              fields: [
                { name: "title", label: "Proposal Title", field_type: "Text", required: true, ferpa_sensitive: false },
                { name: "budget", label: "Budget", field_type: "Number", required: true, ferpa_sensitive: false },
                { name: "rating", label: "Rating", field_type: "Rating", required: false, ferpa_sensitive: false },
                { name: "tags", label: "Tags", field_type: "MultiSelect", required: false, ferpa_sensitive: false, select_options: ["Quantum", "Relativity", "Optics"] },
              ],
            },
          ],
        },
      };

      const onRecordSubmitted = vi.fn();
      render(
        <StandaloneIntakeForm
          app={mockRichApp}
          onRecordSubmitted={onRecordSubmitted}
        />
      );

      // Verify full-screen container
      expect(screen.getByTestId("standalone-intake-form-container")).toBeInTheDocument();
      expect(screen.getByText(/Research Proposals Intake/i)).toBeInTheDocument();

      // Attempt to submit empty required form
      const submitBtn = screen.getByTestId("submit-intake-form-btn");
      fireEvent.click(submitBtn);

      // Validation notice and inline error should appear
      expect(screen.getByText(/Please complete all required fields indicated below/i)).toBeInTheDocument();
      expect(screen.getByText(/Proposal Title is required/i)).toBeInTheDocument();
      expect(onRecordSubmitted).not.toHaveBeenCalled();

      // Enter proposal title
      const titleInput = screen.getByTestId("field-input-title");
      fireEvent.change(titleInput, { target: { value: "Topological Superconductivity" } });

      // Enter budget
      const budgetInput = screen.getByTestId("field-input-budget");
      fireEvent.change(budgetInput, { target: { value: 750000 } });

      // Select rating
      const ratingStar = screen.getByTestId("rating-star-5");
      fireEvent.click(ratingStar);

      // Select multiselect option
      const tagBtn = screen.getByTestId("multiselect-option-Quantum");
      fireEvent.click(tagBtn);

      // Submit valid form
      fireEvent.click(submitBtn);

      // Confirmation card should be rendered
      expect(screen.getByTestId("intake-confirmation-card")).toBeInTheDocument();
      expect(screen.getByText(/Submission Received and Attested/i)).toBeInTheDocument();
      expect(screen.getByText(/sha256:/i)).toBeInTheDocument();
      expect(onRecordSubmitted).toHaveBeenCalledWith(
        "tbl-proposals",
        expect.objectContaining({
          title: "Topological Superconductivity",
          budget: 750000,
        })
      );

      // Reset form via "Submit Another Response"
      const resetBtn = screen.getByTestId("submit-another-btn");
      fireEvent.click(resetBtn);
      expect(screen.queryByTestId("intake-confirmation-card")).not.toBeInTheDocument();
      expect(screen.getByTestId("standalone-intake-form")).toBeInTheDocument();
    });

    it("copies standalone form share URL to clipboard", () => {
      render(<StandaloneIntakeForm app={mockApp} />);

      const copyBtn = screen.getByTestId("copy-form-url-btn");
      expect(copyBtn).toHaveTextContent("Copy Share Link");
      fireEvent.click(copyBtn);
      // Confirms copy button interaction
      expect(copyBtn).toBeInTheDocument();
    });

    it("toggles full-screen mode in AppBuilder and switches to Standalone Forms tab", () => {
      const mockOpenIntake = vi.fn();
      render(
        <AppBuilder
          app={mockApp}
          onBack={vi.fn()}
          onOpenPublishedApp={vi.fn()}
          onOpenIntakeForm={mockOpenIntake}
        />
      );

      // Full-screen mode toggle in top bar
      const fullscreenBtn = screen.getByTestId("fullscreen-mode-toggle");
      expect(fullscreenBtn).toHaveTextContent("Full-Screen");
      fireEvent.click(fullscreenBtn);
      expect(fullscreenBtn).toHaveTextContent("Exit Full-Screen");
      fireEvent.click(fullscreenBtn);
      expect(fullscreenBtn).toHaveTextContent("Full-Screen");

      // Open Standalone Form via top bar button
      const openFormBtn = screen.getByTestId("open-standalone-form-btn");
      fireEvent.click(openFormBtn);
      expect(mockOpenIntake).toHaveBeenCalledWith("tbl-proposals");

      // Switch to Standalone Public Forms tab
      const formsTabBtn = screen.getByTestId("tab-btn-forms");
      fireEvent.click(formsTabBtn);
      expect(screen.getByText(/Standalone Public Intake Forms/i)).toBeInTheDocument();
      expect(screen.getByTestId("launch-public-form-btn")).toBeInTheDocument();
      expect(screen.getByTestId("standalone-intake-form")).toBeInTheDocument();
    });

    it("renders public intake form button in PublishedAppView header", () => {
      const mockOpenIntake = vi.fn();
      render(
        <PublishedAppView
          app={mockApp}
          onOpenBuilder={vi.fn()}
          onOpenIntakeForm={mockOpenIntake}
        />
      );

      const publicIntakeBtn = screen.getByTestId("open-public-intake-form-btn");
      expect(publicIntakeBtn).toBeInTheDocument();
      fireEvent.click(publicIntakeBtn);
      expect(mockOpenIntake).toHaveBeenCalled();
    });
  });

  describe("Authentication, Cedar Authorization & Admin Impersonation Usability", () => {
    it("denies access to /admin when active user is not central_admin and displays Cedar 403 Forbidden", () => {
      window.history.pushState(null, "", "/admin");
      render(<AdminDesk />);

      // Sarah Connor is faculty (default active persona 0), so /admin must show access denied
      const accessDenied = screen.getByTestId("admin-access-denied");
      expect(accessDenied).toBeInTheDocument();
      expect(screen.getByText(/403 Forbidden: Cedar Policy Authorization Required/i)).toBeInTheDocument();
      expect(screen.getByText(/strictly restricts the Administrative Console to/i)).toBeInTheDocument();
    });

    it("authenticates as central_admin via Login modal, granting access to /admin and Identity Hub", () => {
      window.history.pushState(null, "", "/admin");
      render(<AdminDesk />);

      // Click "Sign In / Switch Identity" modal button
      const userBadgeBtn = screen.getByTitle("User Account & Persona Menu");
      fireEvent.click(userBadgeBtn);

      const switchAccountBtn = screen.getByTestId("switch-account-modal-btn");
      fireEvent.click(switchAccountBtn);

      expect(screen.getByTestId("login-modal")).toBeInTheDocument();

      // Sign in as Jordan Lee (central_admin)
      const adminLoginBtn = screen.getByTestId("login-as-jordan.lee@state.edu-btn");
      fireEvent.click(adminLoginBtn);

      // Now /admin should render the Institutional Administrative Console
      expect(screen.queryByTestId("admin-access-denied")).not.toBeInTheDocument();
      expect(screen.getByText(/Institutional Administrative Console/i)).toBeInTheDocument();

      // Switch to Identity & Impersonation Tab
      const impTabBtn = screen.getByTestId("admin-impersonation-tab-btn");
      fireEvent.click(impTabBtn);

      expect(screen.getByTestId("impersonation-panel")).toBeInTheDocument();
      expect(screen.getByText(/Institutional Identity & User Impersonation Hub/i)).toBeInTheDocument();
    });

    it("initiates impersonation of a directory user, displays persistent top banner, and exits back to admin", () => {
      window.history.pushState(null, "", "/admin");
      render(<AdminDesk />);

      // Switch to Jordan Lee
      const userBadgeBtn = screen.getByTitle("User Account & Persona Menu");
      fireEvent.click(userBadgeBtn);
      fireEvent.click(screen.getByTestId("switch-account-modal-btn"));
      fireEvent.click(screen.getByTestId("login-as-jordan.lee@state.edu-btn"));

      // Go to Impersonation tab
      fireEvent.click(screen.getByTestId("admin-impersonation-tab-btn"));

      // Click "Impersonate User" for Dr. Sarah Connor
      const impSarahBtn = screen.getByTestId("impersonate-sarah.connor@state.edu-btn");
      fireEvent.click(impSarahBtn);

      // Verify persistent top banner appears
      const banner = screen.getByTestId("impersonation-banner");
      expect(banner).toBeInTheDocument();
      expect(screen.getByText(/Impersonation Active \(AC-02\)/i)).toBeInTheDocument();
      expect(screen.getAllByText(/Jordan Lee/i).length).toBeGreaterThan(0);
      expect(screen.getAllByText(/Dr\. Sarah Connor/i).length).toBeGreaterThan(0);

      // Click Exit Impersonation button
      const exitBtn = screen.getByTestId("exit-impersonation-btn");
      fireEvent.click(exitBtn);

      // Verify banner disappears and session is restored to admin
      expect(screen.queryByTestId("impersonation-banner")).not.toBeInTheDocument();
      expect(screen.getByText(/Institutional Administrative Console/i)).toBeInTheDocument();
    });

    it("verifies non-admin user menu does not allow unrestricted persona switching", () => {
      window.history.pushState(null, "", "/");
      render(<AdminDesk />);

      // Default user is Dr. Sarah Connor (faculty)
      const userBadgeBtn = screen.getByTitle("User Account & Persona Menu");
      fireEvent.click(userBadgeBtn);

      // Should not have "Switch InCommon Identity" quick switcher list
      expect(screen.queryByText("Switch InCommon Identity")).not.toBeInTheDocument();
      // Should not show admin impersonation hub
      expect(screen.queryByTestId("menu-impersonation-hub-btn")).not.toBeInTheDocument();
    });
  });

  describe("Workspace Sharing Security, Cedar ABAC & Security Configuration Usability", () => {
    it("enforces workspace boundary: restricted workspaces do not appear in list for non-members and direct access triggers Cedar 403 screen", () => {
      // 1. Verify restricted workspaces do NOT appear in the list for unauthorized non-members
      window.history.pushState(null, "", "/");
      render(<AdminDesk />);

      // Dr. Sarah Connor is default active user (Computer Science faculty)
      // Biology Lab & Physics are restricted to their members and must NOT appear in rail or switcher
      expect(screen.queryByTestId("restricted-ws-btn-ws-bio-lab")).not.toBeInTheDocument();
      expect(screen.queryByTestId("workspace-rail-btn-ws-bio-lab")).not.toBeInTheDocument();
      expect(screen.queryByTestId("workspace-rail-btn-ws-physics-optics")).not.toBeInTheDocument();

      // Switcher dropdown options must only contain accessible workspaces
      const switcherSelect = screen.getByTestId("workspace-switcher-select");
      expect(switcherSelect).toBeInTheDocument();
      expect(screen.queryByText(/Biology Research Laboratory/i)).not.toBeInTheDocument();
      expect(screen.getAllByText(/Computer Science & Systems Lab/i).length).toBeGreaterThan(0);
    });

    it("enforces Cedar 403 Forbidden screen upon direct URL navigation to restricted workspace", () => {
      // Non-member navigates directly to restricted Biology Lab workspace URL
      window.history.pushState(null, "", "/workspace/ws-bio-lab");
      render(<AdminDesk />);

      // Cedar 403 Forbidden screen must be shown
      const deniedCard = screen.getByTestId("workspace-access-denied");
      expect(deniedCard).toBeInTheDocument();
      expect(screen.getByText(/403 Forbidden: Cedar Policy Sharing Boundary/i)).toBeInTheDocument();
      expect(screen.getAllByText(/sarah.connor@state.edu/i).length).toBeGreaterThan(0);
      expect(screen.getByText(/Action::"access_workspace"/i)).toBeInTheDocument();

      // Click "Switch to Accessible Workspace" button
      const switchBtn = screen.getByTestId("switch-to-accessible-workspace-btn");
      fireEvent.click(switchBtn);

      // Successfully redirected back to accessible workspace
      expect(screen.queryByTestId("workspace-access-denied")).not.toBeInTheDocument();
      expect(screen.getAllByText(/Computer Science & Systems Lab/i).length).toBeGreaterThan(0);
    });

    it("supports grouping workspaces by organization and department in the rail", () => {
      window.history.pushState(null, "", "/");
      render(<AdminDesk />);

      // Grouped by organization by default
      const orgGroup = screen.getByTestId("workspace-group-college-of-engineering");
      expect(orgGroup).toBeInTheDocument();
      expect(screen.getByTestId("workspace-rail-btn-ws-cs-research")).toBeInTheDocument();

      // Toggle grouping to department
      const groupByDeptBtn = screen.getByTestId("group-by-dept-btn");
      fireEvent.click(groupByDeptBtn);
      expect(screen.getByTestId("workspace-group-computer-science")).toBeInTheDocument();

      // Toggle back to organization
      const groupByOrgBtn = screen.getByTestId("group-by-org-btn");
      fireEvent.click(groupByOrgBtn);
      expect(screen.getByTestId("workspace-group-college-of-engineering")).toBeInTheDocument();
    });

    it("supports pinning and unpinning workspaces with dedicated pinned section and switcher integration", () => {
      window.history.pushState(null, "", "/");
      render(<AdminDesk />);

      // Initially no pinned group
      expect(screen.queryByTestId("pinned-workspaces-group")).not.toBeInTheDocument();

      // Pin the workspace from rail
      const pinBtn = screen.getByTestId("pin-workspace-ws-cs-research");
      fireEvent.click(pinBtn);

      // Pinned group appears in the rail
      const pinnedGroup = screen.getByTestId("pinned-workspaces-group");
      expect(pinnedGroup).toBeInTheDocument();
      expect(pinnedGroup).toHaveTextContent(/Pinned/i);

      // Switcher also includes pinned optgroup
      expect(screen.getByTestId("pinned-optgroup")).toBeInTheDocument();

      // Header button also reflects pinned state
      const headerPinBtn = screen.getByTestId("workspace-header-pin-btn-ws-cs-research");
      expect(headerPinBtn).toHaveTextContent(/Pinned/i);

      // Unpin using header button
      fireEvent.click(headerPinBtn);
      expect(screen.queryByTestId("pinned-workspaces-group")).not.toBeInTheDocument();
      expect(screen.queryByTestId("pinned-optgroup")).not.toBeInTheDocument();
      expect(headerPinBtn).toHaveTextContent(/Pin/i);
    });

    it("allows workspace owner to open Workspace Settings & Security modal and configure details", () => {
      window.history.pushState(null, "", "/");
      render(<AdminDesk />);

      // Dr. Sarah Connor is owner of Computer Science workspace
      const settingsBtn = screen.getByTestId("workspace-settings-btn");
      expect(settingsBtn).toBeInTheDocument();
      fireEvent.click(settingsBtn);

      // Modal opens
      const modal = screen.getByTestId("workspace-settings-modal");
      expect(modal).toBeInTheDocument();
      expect(screen.getByText(/Workspace Security & Configuration/i)).toBeInTheDocument();

      // Check General tab fields
      const nameInput = screen.getByTestId("ws-settings-name-input") as HTMLInputElement;
      expect(nameInput.value).toBe("Computer Science & Systems Lab");

      // Check Sharing & Policy tab
      const accessTabBtn = screen.getByTestId("ws-settings-tab-access");
      fireEvent.click(accessTabBtn);
      const policyPreview = screen.getByTestId("ws-cedar-policy-preview");
      expect(policyPreview).toBeInTheDocument();
      expect(policyPreview.textContent).toContain('Action::"access_workspace"');

      // Check Collaborators tab
      const membersTabBtn = screen.getByTestId("ws-settings-tab-members");
      fireEvent.click(membersTabBtn);
      expect(screen.getByText(/Active Members & Collaborator Roles/i)).toBeInTheDocument();
      expect(screen.getAllByText(/Dr. Sarah Connor/i).length).toBeGreaterThan(0);

      // Add a collaborator
      const eppnSelect = screen.getByTestId("ws-add-member-eppn-input");
      fireEvent.change(eppnSelect, { target: { value: "marcus.vance@state.edu" } });
      const roleSelect = screen.getByTestId("ws-add-member-role-select");
      fireEvent.change(roleSelect, { target: { value: "editor" } });
      const addBtn = screen.getByTestId("ws-add-member-btn");
      fireEvent.click(addBtn);

      // Verify collaborator added
      expect(screen.getAllByText(/Marcus Vance/i).length).toBeGreaterThan(0);

      // Close modal
      const closeBtn = screen.getByTestId("ws-settings-close-btn");
      fireEvent.click(closeBtn);
      expect(screen.queryByTestId("workspace-settings-modal")).not.toBeInTheDocument();
    });

    it("central_admin has supervisory access to all workspaces without 403 restriction", () => {
      window.history.pushState(null, "", "/");
      render(<AdminDesk />);

      // Sign in as Jordan Lee (central_admin)
      const userBadgeBtn = screen.getByTitle("User Account & Persona Menu");
      fireEvent.click(userBadgeBtn);
      fireEvent.click(screen.getByTestId("switch-account-modal-btn"));
      fireEvent.click(screen.getByTestId("login-as-jordan.lee@state.edu-btn"));

      // Switch to Biology Lab workspace
      const switcher = screen.getByTestId("workspace-switcher-select");
      fireEvent.change(switcher, { target: { value: "ws-bio-lab" } });

      // No 403 screen for central_admin; supervisory access is permitted
      expect(screen.queryByTestId("workspace-access-denied")).not.toBeInTheDocument();
      expect(screen.getAllByText(/Biology Research Laboratory/i).length).toBeGreaterThan(0);
      expect(screen.getByText(/Role: ADMIN/i)).toBeInTheDocument();
      // central_admin can manage workspace settings
      expect(screen.getByTestId("workspace-settings-btn")).toBeInTheDocument();
    });

    it("enforces Cedar security rules at the app level and prevents bypassing workspace restrictions in All Campus Apps", () => {
      window.history.pushState(null, "", "/");
      render(<AdminDesk />);

      // Switch to All Campus Apps
      const allAppsBtn = screen.getByText("All Campus Apps");
      fireEvent.click(allAppsBtn);

      // Dr. Sarah Connor (CS) must NOT see apps from restricted workspaces where she is not a member (e.g. bio-lab-inventory)
      expect(screen.queryByText("Biology Lab Equipment & Bioassay Register")).not.toBeInTheDocument();

      // Direct URL navigation to a restricted app must show 403 Forbidden screen
      window.history.pushState(null, "", "/app/bio-lab-inventory");
      render(<AdminDesk />);
      expect(screen.getByTestId("workspace-access-denied")).toBeInTheDocument();
      expect(screen.getByText(/Action::"access_app"/i)).toBeInTheDocument();
    });
  });

  describe("Phase 5: Builder Column Schema Modification", () => {
    it("inserts a field via header menu in builder data-tab grid", () => {
      render(
        <AppBuilder
          app={mockApp}
          onBack={vi.fn()}
          onOpenPublishedApp={vi.fn()}
        />
      );

      // Switch to Data tab
      fireEvent.click(screen.getByTestId("tab-btn-data"));

      // Click column options menu button for budget field
      const menuBtn = screen.getByTestId("column-menu-btn-budget");
      fireEvent.click(menuBtn);

      // Menu popover should open with insert options
      const insertRightBtn = screen.getByTestId("insert-field-right-btn");
      fireEvent.click(insertRightBtn);

      // New field should be inserted with label "Field N" (where N = field count + 1)
      // Original field count was 14, so new field is Field 15
      expect(screen.getAllByText("Field 15").length).toBeGreaterThan(0);
    });

    it("renders evaluateClientFormula output for Formula field in builder grid", () => {
      render(
        <AppBuilder
          app={mockApp}
          onBack={vi.fn()}
          onOpenPublishedApp={vi.fn()}
        />
      );

      // Switch to Data tab
      fireEvent.click(screen.getByTestId("tab-btn-data"));

      // Open column options for indirect_cost Formula field
      const menuBtn = screen.getByTestId("column-menu-btn-indirect_cost");
      fireEvent.click(menuBtn);

      // Edit formula expression to "{budget} * 0.10"
      const formulaInput = screen.getByTestId("formula-expression-input");
      fireEvent.change(formulaInput, { target: { value: "{budget} * 0.10" } });
      fireEvent.click(screen.getByTestId("save-formula-btn"));

      // Formula expression should be displayed under the header
      expect(screen.getByTestId("formula-expression-indirect_cost")).toHaveTextContent("{budget} * 0.10");

      // Grid should re-evaluate and display 5,000 for APP-001 (450,000 * 0.10)
      expect(screen.getByText("$45,000")).toBeInTheDocument();
    });

    it("renames field, preserving key when data exists and updating key when empty", () => {
      render(
        <AppBuilder
          app={mockApp}
          onBack={vi.fn()}
          onOpenPublishedApp={vi.fn()}
        />
      );
      fireEvent.click(screen.getByTestId("tab-btn-data"));

      // Rename budget field (which has data)
      fireEvent.click(screen.getByTestId("column-menu-btn-budget"));
      const input = screen.getByTestId("rename-field-input");
      fireEvent.change(input, { target: { value: "Approved Budget" } });
      fireEvent.click(screen.getByTestId("save-rename-btn"));

      // Label updated
      expect(screen.getAllByText("Approved Budget").length).toBeGreaterThan(0);
      // Notice shown that key is kept
      expect(screen.getByTestId("rename-notice")).toHaveTextContent("Key 'budget' kept");
    });

    it("changes field type and clears stored records when changing to Formula", () => {
      render(
        <AppBuilder
          app={mockApp}
          onBack={vi.fn()}
          onOpenPublishedApp={vi.fn()}
        />
      );
      fireEvent.click(screen.getByTestId("tab-btn-data"));

      // Change status field to Formula
      fireEvent.click(screen.getByTestId("column-menu-btn-status"));
      const typeSelect = screen.getByTestId("change-type-select");
      fireEvent.change(typeSelect, { target: { value: "Formula" } });

      // Popover shows Formula Expression input now that type is Formula
      expect(screen.getByTestId("formula-expression-input")).toBeInTheDocument();
    });

    it("deletes non-primary field and blocks deletion of primary field", () => {
      const alertMock = vi.spyOn(window, "alert").mockImplementation(() => {});
      render(
        <AppBuilder
          app={mockApp}
          onBack={vi.fn()}
          onOpenPublishedApp={vi.fn()}
        />
      );
      fireEvent.click(screen.getByTestId("tab-btn-data"));

      // Try deleting primary field 'title'
      fireEvent.click(screen.getByTestId("column-menu-btn-title"));
      const deletePrimaryBtn = screen.getByTestId("delete-field-btn");
      expect(deletePrimaryBtn).toBeDisabled();
      fireEvent.click(deletePrimaryBtn);
      // Not deleted
      expect(screen.getAllByText("Proposal Title").length).toBeGreaterThan(0);

      // Now delete a non-primary field: department
      fireEvent.click(screen.getByTestId("column-menu-btn-department"));
      const deleteDeptBtn = screen.getByTestId("delete-field-btn");
      expect(deleteDeptBtn).not.toBeDisabled();
      fireEvent.click(deleteDeptBtn);

      // Department should no longer be in headers
      expect(screen.queryByTestId("column-menu-btn-department")).not.toBeInTheDocument();
      alertMock.mockRestore();
    });
  });

  describe("Phase 6: Linked Field & Airtable-style Link Characteristics Usability", () => {
    it("adds a linked field via + Add Field button with Airtable-style link characteristics", () => {
      render(
        <AppBuilder
          app={mockApp}
          onBack={vi.fn()}
          onOpenPublishedApp={vi.fn()}
        />
      );
      fireEvent.click(screen.getByTestId("tab-btn-data"));

      // Click "+ Add Field" header button
      const addFieldBtn = screen.getByTestId("add-column-header-btn");
      expect(addFieldBtn).toBeInTheDocument();
      fireEvent.click(addFieldBtn);

      // Link characteristics modal opens
      expect(screen.getByTestId("link-characteristics-modal")).toBeInTheDocument();

      // Configure field label
      const labelInput = screen.getByTestId("link-field-label-input");
      fireEvent.change(labelInput, { target: { value: "Assigned Investigator" } });

      // Configure target table
      const tableSelect = screen.getByTestId("link-target-table-select");
      fireEvent.change(tableSelect, { target: { value: "tbl-investigators" } });

      // Configure cardinality: multiple records
      const multipleBtn = screen.getByTestId("link-cardinality-multiple");
      fireEvent.click(multipleBtn);

      // Configure display field & label override
      const displaySelect = screen.getByTestId("link-display-field-select");
      fireEvent.change(displaySelect, { target: { value: "name" } });

      const overrideInput = screen.getByTestId("link-label-override-input");
      fireEvent.change(overrideInput, { target: { value: "Lead PI: " } });

      // Enable and configure filter
      const filterCheckbox = screen.getByTestId("link-filter-enable-checkbox");
      fireEvent.click(filterCheckbox);

      const filterFieldSelect = screen.getByTestId("link-filter-field-select");
      fireEvent.change(filterFieldSelect, { target: { value: "department" } });

      const filterOpSelect = screen.getByTestId("link-filter-operator-select");
      fireEvent.change(filterOpSelect, { target: { value: "equals" } });

      const filterValInput = screen.getByTestId("link-filter-value-input");
      fireEvent.change(filterValInput, { target: { value: "Physics" } });

      // Save linked field
      fireEvent.click(screen.getByTestId("save-linked-field-btn"));

      // Modal closed, new linked column exists
      expect(screen.queryByTestId("link-characteristics-modal")).not.toBeInTheDocument();
      expect(screen.getAllByText("Assigned Investigator").length).toBeGreaterThan(0);
    });

    it("opens link characteristics configuration modal from column menu for existing relation field", () => {
      render(
        <AppBuilder
          app={mockApp}
          onBack={vi.fn()}
          onOpenPublishedApp={vi.fn()}
        />
      );
      fireEvent.click(screen.getByTestId("tab-btn-data"));

      // Open column menu for lead_investigator_id (Relation field)
      const menuBtn = screen.getByTestId("column-menu-btn-lead_investigator_id");
      fireEvent.click(menuBtn);

      // Configure Link Characteristics button is present
      const configBtn = screen.getByTestId("configure-link-btn");
      expect(configBtn).toBeInTheDocument();
      fireEvent.click(configBtn);

      // Modal opens with pre-populated values
      expect(screen.getByTestId("link-characteristics-modal")).toBeInTheDocument();
      const labelInput = screen.getByTestId("link-field-label-input") as HTMLInputElement;
      expect(labelInput.value).toBe("Lead Investigator");

      // Close modal
      fireEvent.click(screen.getByTestId("close-link-modal-btn"));
      expect(screen.queryByTestId("link-characteristics-modal")).not.toBeInTheDocument();
    });
  });
});
