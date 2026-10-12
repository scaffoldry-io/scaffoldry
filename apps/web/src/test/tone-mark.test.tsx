import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import "@testing-library/jest-dom";

import { ToneMark, nameWithTones, toneLabels, AppliedTone } from "../ui";

const tones: AppliedTone[] = [
  { id: "r1", label: "At risk", tone: "danger", target: "row" },
  { id: "r2", label: "Large", tone: "warning", target: "field", field: "amount" },
];

describe("Color rules — a tone is never only a color", () => {
  it("names a row's rules in its accessible name and shows each as an icon and a label", () => {
    render(<ToneMark tones={tones} />);
    // The group is named for the rule, so a screen reader hears it.
    const group = screen.getByRole("group", { name: "Marked by: At risk" });
    expect(group).toBeInTheDocument();
    // The label is text in the page, not only a style. The icon is decoration.
    expect(screen.getByText("At risk")).toBeVisible();
    expect(group.querySelector('[aria-hidden="true"]')).not.toBeNull();
    // A rule that colors a field is not shown on the row.
    expect(screen.queryByText("Large")).toBeNull();
  });

  it("marks a field with its own rules", () => {
    render(<ToneMark tones={tones} field="amount" />);
    expect(screen.getByRole("group", { name: "Marked by: Large" })).toBeInTheDocument();
    expect(toneLabels(tones, "amount")).toEqual(["Large"]);
    expect(toneLabels(tones)).toEqual(["At risk"]);
  });

  it("puts every rule label into a row's name for assistive technology", () => {
    expect(nameWithTones("Row 115", tones)).toBe("Row 115. At risk, Large");
    expect(nameWithTones("Row 2", undefined)).toBe("Row 2");
  });

  it("shows nothing for a row no rule marks", () => {
    const { container } = render(<ToneMark tones={[]} />);
    expect(container).toBeEmptyDOMElement();
  });
});
