import "@testing-library/jest-dom";
import { describe, it, expect } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/react";
import { BarChart, LineChart, PieChart, StatChart, SummaryTable, niceTicks, unsavedViewAddress, readUnsavedView } from "../ui/charts";

const months = ["January", "February", "March", "April"];

describe("BarChart", () => {
  const props = { title: "Applications by month", categories: months, series: [{ name: "Applications", values: [12, 30, 42, 18] }], unit: "applications", xLabel: "Month", yLabel: "Applications" };

  it("renders one rect per group with a text label and a pattern fill, and the table holds the same numbers", () => {
    const { container } = render(<BarChart {...props} />);
    const bars = container.querySelectorAll("rect[data-point]");
    expect(bars).toHaveLength(4);
    // Each bar is filled with a pattern, not only a color.
    bars.forEach((bar) => expect(bar.getAttribute("fill")).toMatch(/^url\(#.+\)$/));
    // Each bar has its value written beside it as text, and the axes are named and ticked.
    expect([...container.querySelectorAll("text[data-value]")].map((t) => t.textContent)).toEqual(["12", "30", "42", "18"]);
    expect(screen.getByText("Month")).toBeInTheDocument();
    expect(screen.getAllByText("Applications").length).toBeGreaterThan(0);
    expect(container.querySelectorAll("[data-tick]").length).toBeGreaterThan(2);
    // The legend is text.
    expect(screen.getByRole("list", { name: "Legend" })).toHaveTextContent("Applications");

    // The table is the exact data.
    fireEvent.click(screen.getByRole("button", { name: "View as table" }));
    const table = screen.getByRole("table", { name: "Applications by month" });
    const rows = within(table).getAllByRole("row").slice(1).map((r) => within(r).getAllByRole("cell").map((c) => c.textContent));
    expect(rows).toEqual([["January", "12"], ["February", "30"], ["March", "42"], ["April", "18"]]);
    fireEvent.click(screen.getByRole("button", { name: "View as chart" }));
    expect(container.querySelectorAll("rect[data-point]")).toHaveLength(4);
  });

  it("gives each series its own pattern, so series are told apart without color", () => {
    const { container } = render(<BarChart {...props} series={[{ name: "Admitted", values: [5, 6, 7, 8] }, { name: "Waitlisted", values: [1, 2, 3, 4] }]} />);
    const fills = new Set([...container.querySelectorAll("rect[data-point]")].map((r) => r.getAttribute("fill")));
    expect(fills.size).toBe(2);
    expect(container.querySelectorAll("pattern").length).toBeGreaterThanOrEqual(2);
  });

  it("stacks series when asked", () => {
    const { container } = render(<BarChart {...props} stacked series={[{ name: "Admitted", values: [5, 6, 7, 8] }, { name: "Waitlisted", values: [1, 2, 3, 4] }]} />);
    const bars = [...container.querySelectorAll("rect[data-point]")];
    expect(bars).toHaveLength(8);
    const first = bars.filter((b) => b.getAttribute("data-category") === "0");
    // The second series sits on the first: its top is above the first one's top.
    const top = (b: Element) => Number(b.getAttribute("y"));
    expect(top(first[1])).toBeLessThan(top(first[0]));
  });

  it("moves through the points with the arrow keys and announces each", () => {
    render(<BarChart {...props} />);
    const chart = screen.getByRole("group", { name: /Applications by month/ });
    const live = screen.getByRole("status");
    expect(live).toHaveTextContent("");
    chart.focus();
    fireEvent.keyDown(chart, { key: "ArrowRight" });
    expect(live).toHaveTextContent("January: 12 applications");
    fireEvent.keyDown(chart, { key: "ArrowRight" });
    expect(live).toHaveTextContent("February: 30 applications");
    fireEvent.keyDown(chart, { key: "ArrowRight" });
    fireEvent.keyDown(chart, { key: "ArrowRight" });
    expect(live).toHaveTextContent("April: 18 applications");
    fireEvent.keyDown(chart, { key: "ArrowRight" });
    expect(live).toHaveTextContent("April: 18 applications");
    fireEvent.keyDown(chart, { key: "ArrowLeft" });
    expect(live).toHaveTextContent("March: 42 applications");
    fireEvent.keyDown(chart, { key: "Home" });
    expect(live).toHaveTextContent("January: 12 applications");
    fireEvent.keyDown(chart, { key: "End" });
    expect(live).toHaveTextContent("April: 18 applications");
  });

  it("names the series in the announcement when there are several, and Up and Down change series", () => {
    render(<BarChart {...props} series={[{ name: "Admitted", values: [5, 6, 7, 8] }, { name: "Waitlisted", values: [1, 2, 3, 4] }]} />);
    const chart = screen.getByRole("group", { name: /Applications by month/ });
    const live = screen.getByRole("status");
    fireEvent.keyDown(chart, { key: "ArrowRight" });
    expect(live).toHaveTextContent("January, Admitted: 5 applications");
    fireEvent.keyDown(chart, { key: "ArrowDown" });
    expect(live).toHaveTextContent("January, Waitlisted: 1 applications");
    fireEvent.keyDown(chart, { key: "ArrowUp" });
    expect(live).toHaveTextContent("January, Admitted: 5 applications");
  });

  it("shows an empty state when there is nothing to chart", () => {
    render(<BarChart {...props} categories={[]} series={[{ name: "Applications", values: [] }]} />);
    expect(screen.getByText("Nothing to chart yet")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "View as table" })).toBeNull();
  });
});

describe("LineChart", () => {
  const props = { title: "Applications per month", categories: months, series: [{ name: "Applications", values: [12, 30, 42, 18] }], unit: "applications" };

  it("draws a line through every point and an area when asked", () => {
    const { container, rerender } = render(<LineChart {...props} />);
    expect(container.querySelectorAll("circle[data-point]")).toHaveLength(4);
    expect(container.querySelectorAll("path[data-line]")).toHaveLength(1);
    expect(container.querySelectorAll("path[data-area]")).toHaveLength(0);
    rerender(<LineChart {...props} area />);
    expect(container.querySelectorAll("path[data-area]")).toHaveLength(1);
  });

  it("announces the point the arrow keys reach", () => {
    render(<LineChart {...props} />);
    const chart = screen.getByRole("group", { name: /Applications per month/ });
    fireEvent.keyDown(chart, { key: "ArrowRight" });
    fireEvent.keyDown(chart, { key: "ArrowRight" });
    expect(screen.getByRole("status")).toHaveTextContent("February: 30 applications");
  });

  it("tells series apart by dash as well as by color", () => {
    const { container } = render(<LineChart {...props} series={[{ name: "Admitted", values: [1, 2, 3, 4] }, { name: "Waitlisted", values: [4, 3, 2, 1] }]} />);
    const dashes = new Set([...container.querySelectorAll("path[data-line]")].map((p) => p.getAttribute("stroke-dasharray")));
    expect(dashes.size).toBe(2);
  });
});

describe("PieChart", () => {
  it("shows 11 slices and Other for a pie over 20 groups", () => {
    const slices = Array.from({ length: 20 }, (_, i) => ({ label: `Program ${i + 1}`, value: 20 - i }));
    const { container } = render(<PieChart title="Applications by program" slices={slices} unit="applications" />);
    expect(container.querySelectorAll("path[data-point]")).toHaveLength(12);
    const legend = screen.getByRole("list", { name: "Legend" });
    expect(within(legend).getAllByRole("listitem")).toHaveLength(12);
    expect(within(legend).getByText(/Other/)).toBeInTheDocument();
    // Other is the nine smallest groups: 9+8+...+1 = 45.
    expect(within(legend).getByText(/Other/).closest("li")).toHaveTextContent("45");
    expect(within(legend).queryByText(/Program 12$/)).toBeNull();
  });

  it("keeps up to 12 groups whole", () => {
    const slices = Array.from({ length: 12 }, (_, i) => ({ label: `P${i + 1}`, value: i + 1 }));
    const { container } = render(<PieChart title="By program" slices={slices} unit="applications" />);
    expect(container.querySelectorAll("path[data-point]")).toHaveLength(12);
    expect(screen.queryByText(/Other/)).toBeNull();
  });

  it("draws a donut with a hole, announces slices, and has a table", () => {
    const { container } = render(<PieChart title="By status" donut slices={[{ label: "Admitted", value: 30 }, { label: "Waitlisted", value: 10 }]} unit="applications" />);
    expect(container.querySelector("circle[data-hole]")).not.toBeNull();
    const chart = screen.getByRole("group", { name: /By status/ });
    fireEvent.keyDown(chart, { key: "ArrowRight" });
    expect(screen.getByRole("status")).toHaveTextContent("Admitted: 30 applications, 75%");
    fireEvent.keyDown(chart, { key: "ArrowRight" });
    expect(screen.getByRole("status")).toHaveTextContent("Waitlisted: 10 applications, 25%");
    fireEvent.click(screen.getByRole("button", { name: "View as table" }));
    expect(within(screen.getByRole("table")).getAllByRole("row")).toHaveLength(3);
  });

  it("patterns the slices, not only colors them", () => {
    const { container } = render(<PieChart title="By status" slices={[{ label: "A", value: 1 }, { label: "B", value: 2 }, { label: "C", value: 3 }]} unit="x" />);
    const fills = new Set([...container.querySelectorAll("path[data-point]")].map((p) => p.getAttribute("fill")));
    expect(fills.size).toBe(3);
    fills.forEach((f) => expect(f).toMatch(/^url\(#.+\)$/));
  });
});

describe("StatChart", () => {
  it("renders both numbers and the difference in words", () => {
    render(<StatChart label="Applications this term" value={42} unit="applications" comparison={{ label: "last term", value: 30 }} />);
    expect(screen.getByText("42")).toBeInTheDocument();
    expect(screen.getByText("30")).toBeInTheDocument();
    expect(screen.getByText("Up 12 from 30 last term (40%)")).toBeInTheDocument();
  });

  it("says down, and says no change", () => {
    const { rerender } = render(<StatChart label="Applications" value={24} unit="applications" comparison={{ label: "last term", value: 30 }} />);
    expect(screen.getByText("Down 6 from 30 last term (20%)")).toBeInTheDocument();
    rerender(<StatChart label="Applications" value={30} unit="applications" comparison={{ label: "last term", value: 30 }} />);
    expect(screen.getByText("No change from 30 last term")).toBeInTheDocument();
  });

  it("does not divide by zero", () => {
    render(<StatChart label="Applications" value={5} unit="applications" comparison={{ label: "last term", value: 0 }} />);
    expect(screen.getByText("Up 5 from 0 last term")).toBeInTheDocument();
  });

  it("stands alone with no comparison", () => {
    render(<StatChart label="Applications" value={1234567} unit="applications" />);
    expect(screen.getByText("1,234,567")).toBeInTheDocument();
    expect(screen.queryByText(/from/)).toBeNull();
  });
});

describe("axes", () => {
  it("picks round tick values that cover the data", () => {
    expect(niceTicks(0, 42)).toEqual([0, 10, 20, 30, 40, 50]);
    expect(niceTicks(0, 7)).toEqual([0, 2, 4, 6, 8]);
    expect(niceTicks(0, 0)).toEqual([0, 1]);
    expect(niceTicks(0, 1_000_000)).toEqual([0, 200000, 400000, 600000, 800000, 1000000]);
  });
});

describe("a summary row opens the grid on its group", () => {
  it("carries the group as a filter in the address, in an unsaved view", () => {
    const address = unsavedViewAddress("registrar", "students", [{ field: "program", value: "biology" }, { field: "stage", value: "" }]);
    expect(address.startsWith("?unsaved=")).toBe(true);
    const view = readUnsavedView(address)!;
    expect(view.table).toBe("students");
    expect(view.filter.conjunction).toBe("And");
    expect(view.filter.clauses.map((c) => [c.field_name, c.operator, c.value])).toEqual([["program", "Equals", "biology"], ["stage", "IsEmpty", ""]]);
    expect(readUnsavedView("?other=1")).toBeNull();
    expect(readUnsavedView("?unsaved=not-json")).toBeNull();
  });
});

describe("SummaryTable", () => {
  const summary = {
    columns: ["program", "count", "sum of amount"],
    groups: [
      { key: ["biology"], values: [12, 1200.5] },
      { key: [""], values: [3, null] },
    ],
    has_more: true,
  };

  it("lists the groups and their measures, and each group opens the grid on its rows", () => {
    render(<SummaryTable summary={summary} appSlug="registrar" tableId="students" groupBy={["program"]} gridPath="/apps/registrar" />);
    const rows = within(screen.getByRole("table", { name: "Summary" })).getAllByRole("row").slice(1).map((r) => within(r).getAllByRole("cell").map((c) => c.textContent));
    expect(rows).toEqual([["biology", "12", "1,200.5"], ["(blank)", "3", ""]]);
    const link = screen.getByRole("link", { name: "biology" });
    const view = readUnsavedView(link.getAttribute("href")!.replace("/apps/registrar", ""))!;
    expect(view.filter.clauses).toEqual([{ id: "group-0", field_name: "program", operator: "Equals", value: "biology" }]);
    // The group with no value opens on the rows that have none.
    const blank = readUnsavedView(screen.getByRole("link", { name: "(blank)" }).getAttribute("href")!.replace("/apps/registrar", ""))!;
    expect(blank.filter.clauses[0].operator).toBe("IsEmpty");
  });

  it("says when there are more groups than it shows", () => {
    render(<SummaryTable summary={summary} appSlug="registrar" tableId="students" groupBy={["program"]} gridPath="/apps/registrar" />);
    expect(screen.getByRole("status")).toHaveTextContent("Showing the first 200 groups. There are more.");
  });
});
