import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { AccessRulesTab, ruleSentence, predInWords } from "../AccessRulesTab";
import { apiClient } from "../api";
import { RegisteredApp, AppTable } from "../types";

const mockTable: AppTable = {
  id: "students",
  name: "Students",
  slug: "students",
  fields: [
    { name: "advisor", label: "Advisor", field_type: "Text", required: false, ferpa_sensitive: false, indexed: true },
    { name: "gpa", label: "GPA", field_type: "Number", required: false, ferpa_sensitive: false },
    { name: "name", label: "Name", field_type: "Text", required: false, ferpa_sensitive: false },
  ],
  records: [],
};

const mockApp: RegisteredApp = {
  id: "app-1",
  slug: "advising",
  title: "Student Advising",
  manifest: {
    slug: "advising",
    title: "Student Advising",
    description: "App description",
    organization_code: "PHYS",
    department: "Physics",
    custom_domain_verified: false,
    views: [],
    ceds_mappings: {},
    access: {
      rows: [
        {
          id: "r1",
          table_id: "students",
          ops: ["Read"],
          visible_when: {
            Cmp: {
              field: "advisor",
              op: "Eq",
              value: { Person: "Address" },
            },
          },
          except_roles: ["owner", "admin"],
          message: "Only your own advisees are visible.",
        },
      ],
      columns: [],
    },
  },
};

describe("Phase 5: Access Rules Web UI", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("formats sentence for Eq advisor = Address correctly", () => {
    const rule = mockApp.manifest.access!.rows[0];
    const sentence = ruleSentence(rule, mockTable.fields);
    expect(sentence).toBe("People who are not owners or admins can see only rows where Advisor is themselves.");
  });

  it("the rule builder produces the JSON the server accepts", async () => {
    let savedApp: RegisteredApp | null = null;
    render(
      <AccessRulesTab
        app={mockApp}
        tables={[mockTable]}
        onSaveProposal={(app) => {
          savedApp = app;
        }}
      />
    );

    // Initial rule is listed with sentence
    expect(screen.getByText("People who are not owners or admins can see only rows where Advisor is themselves.")).toBeInTheDocument();

    // Open builder
    fireEvent.click(screen.getByTestId("add-rule-btn"));
    expect(screen.getByTestId("rule-builder")).toBeInTheDocument();

    // Check operations: select "change rows" (Update) as well
    fireEvent.click(screen.getByTestId("op-checkbox-Update"));

    // Enter message
    fireEvent.change(screen.getByTestId("rule-message-input"), {
      target: { value: "Advisor can also update records" },
    });
    expect(screen.getByTestId("message-char-count")).toHaveTextContent("31/200");

    // Add rule
    fireEvent.click(screen.getByTestId("save-rule-btn"));

    // Now two rules should be in the list
    expect(screen.queryByTestId("rule-builder")).not.toBeInTheDocument();

    // Click Save proposal
    fireEvent.click(screen.getByTestId("save-access-proposal-btn"));

    expect(savedApp).not.toBeNull();
    const rows = savedApp!.manifest.access!.rows;
    expect(rows.length).toBe(2);
    const added = rows[1];
    expect(added.table_id).toBe("students");
    expect(added.ops).toContain("Read");
    expect(added.ops).toContain("Update");
    expect(added.visible_when).toEqual({
      Cmp: {
        field: "advisor",
        op: "Eq",
        value: { Person: "Address" },
      },
    });
    expect(added.except_roles).toEqual(["owner", "admin"]);
    expect(added.message).toBe("Advisor can also update records");
  });

  it("the test panel shows counts matching the preview endpoint", async () => {
    vi.spyOn(apiClient, "previewAccess").mockResolvedValue({
      can_see: 12,
      can_add: 0,
      can_change: 12,
      can_remove: 0,
      total_rows: 4031,
      hidden_columns: [],
    });

    render(<AccessRulesTab app={mockApp} tables={[mockTable]} />);

    fireEvent.change(screen.getByTestId("test-person-name-input"), {
      target: { value: "Dr. Rivera" },
    });
    fireEvent.change(screen.getByTestId("test-person-eppn-input"), {
      target: { value: "rivera@state.edu" },
    });

    fireEvent.click(screen.getByTestId("run-preview-btn"));

    await waitFor(() => {
      expect(screen.getByTestId("preview-counts-result")).toBeInTheDocument();
    });

    expect(
      screen.getByText("Dr. Rivera would see 12 of 4,031 rows and could change 12.")
    ).toBeInTheDocument();
  });

  it("problems show as a banner in words", async () => {
    const brokenApp: RegisteredApp = {
      ...mockApp,
      manifest: {
        ...mockApp.manifest,
        access: {
          rows: [
            {
              id: "r_broken",
              table_id: "students",
              ops: ["Read"],
              visible_when: {
                And: [
                  { Cmp: { field: "advisor", op: "Eq", value: { Literal: "x" } } },
                  { Cmp: { field: "advisor", op: "Ne", value: { Literal: "x" } } },
                ],
              },
              except_roles: [],
            },
          ],
          columns: [],
        },
      },
    };

    render(<AccessRulesTab app={brokenApp} tables={[mockTable]} />);

    const bannerContainer = screen.getByTestId("problems-banner-container");
    expect(bannerContainer).toBeInTheDocument();
    expect(bannerContainer).toHaveTextContent(/never_true: Rule 'r_broken' contains a contradiction/);
  });
});
