import React from "react";
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import "@testing-library/jest-dom";
import { DataGrid } from "../DataGrid";
import type { FieldSpec } from "../types";

const text: FieldSpec = { name: "name", label: "Name", field_type: "Text", required: false, ferpa_sensitive: false };
const amount: FieldSpec = { name: "amount", label: "Amount", field_type: "Number", required: false, ferpa_sensitive: false };
const rows = [{ id: "r1", name: "Alpha", amount: 5 }];

describe("DataGrid footer", () => {
  it("has no blank summary row when no column has a summary", () => {
    const { container } = render(<DataGrid fields={[text]} records={rows} onPatch={() => {}} />);
    expect(container.querySelector("tfoot")).toBeNull();
    expect(screen.queryByText("—")).not.toBeInTheDocument();
  });

  it("still totals a number column", () => {
    render(<DataGrid fields={[text, amount]} records={rows} onPatch={() => {}} />);
    expect(screen.getByTestId("summary-amount")).toHaveTextContent("5");
  });
});
