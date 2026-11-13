import "@testing-library/jest-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { Dashboard, type DashboardWidget } from "../Dashboard";
import { apiClient, type AppView } from "../api";

vi.mock("../PageFrame", () => ({
  PageFrame: ({ pageId, selection }: { pageId: string; selection?: string | null }) => (
    <p data-testid={`frame-${pageId}`}>{selection === undefined || selection === null ? "no selection" : `selection ${selection}`}</p>
  ),
}));

const widget = (id: string, title: string, x: number, y: number, w: number, h: number, body: DashboardWidget["body"], extra: Partial<DashboardWidget> = {}): DashboardWidget => ({
  id, title, x, y, w, h, body, ...extra,
});

const widgets: DashboardWidget[] = [
  widget("list", "Students", 0, 0, 6, 4, { type: "view", view_id: "v1" }),
  widget("chart", "By program", 6, 0, 6, 4, { type: "chart", view_id: "v2" }, { select_by: { source_widget: "list", source_field: "program", target_field: "program" } }),
  widget("total", "Total students", 0, 4, 3, 2, { type: "stat", table_id: "students", fn: "count" }),
  widget("note", "Read me", 3, 4, 3, 2, { type: "text", markdown: "Read **this** first" }),
  widget("board", "Board", 6, 4, 6, 4, { type: "page", page_id: "board" }, { select_by: { source_widget: "list", source_field: "program", target_field: "program" } }),
];

const dashboard = (list: DashboardWidget[] = widgets): AppView => ({
  id: "d1", app_slug: "grants", table_id: null, name: "Dean", kind: "dashboard", ownership: "collaborative", owner: null,
  config: { columns: 12, widgets: list }, version: 3, created_by: "a", created_at: "", updated_by: "a", updated_at: "",
});

const rows = [
  { id: "r1", data: { name: "Ada", program: "p1" } },
  { id: "r2", data: { name: "Bo", program: "p2" } },
];

function serve(opts: { select?: string }, id: string) {
  const base = { widget_id: id, selected: opts.select ? { field: "program", value: opts.select } : null };
  if (id === "list") return { ...base, kind: "view", data: { records: rows } };
  if (id === "chart") {
    const groups = [{ key: ["p1"], values: [8] }, { key: ["p2"], values: [6] }].filter((g) => !opts.select || g.key[0] === opts.select);
    return { ...base, kind: "chart", data: { summary: { columns: ["program", "count"], groups, has_more: false } } };
  }
  if (id === "total") return { ...base, kind: "stat", data: { stat: { fn: "count", field: null, value: 40 } } };
  return { ...base, kind: id === "note" ? "text" : "page", data: {} };
}

function setWidth(px: number) {
  Object.defineProperty(window, "innerWidth", { configurable: true, value: px });
  act(() => {
    window.dispatchEvent(new Event("resize"));
  });
}

let ask: ReturnType<typeof vi.spyOn>;
beforeEach(() => {
  setWidth(1200);
  ask = vi.spyOn(apiClient, "dashboardWidget").mockImplementation(async (_app: string, _dash: string, id: string, opts: { select?: string } = {}) => serve(opts, id) as never);
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("a dashboard", () => {
  it("draws each widget as a landmark named by its title", async () => {
    render(<Dashboard appSlug="grants" view={dashboard()} />);
    for (const name of ["Students", "By program", "Total students", "Read me", "Board"]) {
      expect(await screen.findByRole("region", { name })).toBeInTheDocument();
    }
    expect(await screen.findByText("Ada")).toBeInTheDocument();
    const stat = within(screen.getByRole("region", { name: "Total students" }));
    expect(await stat.findByText("40")).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: "Read me" })).getByText("this")).toBeInTheDocument();
  });

  // Test 1 of the brief, from the screen: the browser only passes the selected value. The server filters.
  it("selecting a row asks the server for the linked widgets with the selection, and clearing removes it", async () => {
    render(<Dashboard appSlug="grants" view={dashboard()} />);
    fireEvent.click(await screen.findByRole("button", { name: "Select row 1 of Students" }));
    await waitFor(() => expect(ask).toHaveBeenCalledWith("grants", "d1", "chart", { select: "p1" }));
    const chart = screen.getByRole("region", { name: "By program" });
    expect(await within(chart).findByText("Only program = p1")).toBeInTheDocument();
    // The source widget has no link, so it is not asked again with a selection.
    expect(ask).not.toHaveBeenCalledWith("grants", "d1", "list", { select: "p1" });

    fireEvent.click(screen.getByRole("button", { name: "Clear selection in Students" }));
    await waitFor(() => expect(ask).toHaveBeenLastCalledWith("grants", "d1", "chart", {}));
    expect(within(chart).queryByText("Only program = p1")).not.toBeInTheDocument();
  });

  it("gives a page widget the selected record's id and nothing else", async () => {
    render(<Dashboard appSlug="grants" view={dashboard()} />);
    expect(await screen.findByTestId("frame-board")).toHaveTextContent("no selection");
    fireEvent.click(await screen.findByRole("button", { name: "Select row 2 of Students" }));
    expect(await screen.findByText("selection r2")).toBeInTheDocument();
  });

  // Test 5 of the brief, part 2: below 900 px the widgets stack in reading order.
  it("stacks the widgets in reading order below 900 px, and places them on the grid above it", async () => {
    const jumbled = [widgets[3], widgets[1], widgets[2], widgets[0]];
    render(<Dashboard appSlug="grants" view={dashboard(jumbled)} />);
    const names = () => screen.getAllByRole("region").map((r) => r.getAttribute("aria-label") ?? r.querySelector("h2")?.textContent);
    await screen.findByText("Ada");
    expect(names()).toEqual(["Students", "By program", "Total students", "Read me"]);
    expect(screen.getByRole("region", { name: "By program" }).style.gridColumn).toBe("7 / span 6");

    setWidth(800);
    await waitFor(() => expect(screen.getByRole("region", { name: "By program" }).style.gridColumn).toBe(""));
    expect(names()).toEqual(["Students", "By program", "Total students", "Read me"]);
  });
});

describe("editing the layout", () => {
  // Two widgets side by side, with room below them to move into.
  const pair = [widgets[0], widgets[1]];
  const edit = async (list = pair) => {
    render(<Dashboard appSlug="grants" view={dashboard(list)} canEdit />);
    fireEvent.click(await screen.findByRole("button", { name: "Edit layout" }));
  };
  const handle = (name: string) => screen.getByRole("group", { name: `Move or resize ${name}` });
  const input = (label: string) => (screen.getByLabelText(label) as HTMLInputElement).value;

  // Test 5 of the brief, part 1: arrow keys move, Shift with arrows resizes, and the numbers agree.
  it("moves a widget with the arrow keys, resizes it with Shift and the arrows, and the numeric inputs agree", async () => {
    await edit();
    fireEvent.focus(handle("By program"));
    expect([input("X"), input("Y"), input("Width"), input("Height")]).toEqual(["6", "0", "6", "4"]);

    fireEvent.keyDown(handle("By program"), { key: "ArrowDown" });
    expect([input("X"), input("Y")]).toEqual(["6", "1"]);
    fireEvent.keyDown(handle("By program"), { key: "ArrowDown", shiftKey: true });
    expect([input("Width"), input("Height")]).toEqual(["6", "5"]);
    fireEvent.keyDown(handle("By program"), { key: "ArrowLeft", shiftKey: true });
    expect([input("Width"), input("Height")]).toEqual(["5", "5"]);
    // The numbers set the exact values, and the keys carry on from them.
    fireEvent.change(screen.getByLabelText("X"), { target: { value: "7" } });
    expect(input("X")).toBe("7");
    fireEvent.keyDown(handle("By program"), { key: "ArrowRight" });
    expect(input("X")).toBe("7");
    expect(screen.getByTestId("layout-note")).toHaveTextContent(/past the right edge/);
  });

  it("refuses a move that would overlap another widget, and says which", async () => {
    await edit();
    fireEvent.focus(handle("By program"));
    fireEvent.keyDown(handle("By program"), { key: "ArrowLeft" });
    expect(input("X")).toBe("6");
    expect(screen.getByTestId("layout-note")).toHaveTextContent(/overlap.*Students/i);
  });

  it("saves the layout with the version it started from, and Cancel puts it back", async () => {
    const save = vi.spyOn(apiClient, "updateView").mockResolvedValue({ view: dashboard(), index_state: "ready" } as never);
    await edit();
    fireEvent.focus(handle("By program"));
    fireEvent.keyDown(handle("By program"), { key: "ArrowDown" });
    fireEvent.click(screen.getByRole("button", { name: "Save layout" }));
    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    const [app, id, payload] = save.mock.calls[0] as [string, string, { name: string; version: number; config: { columns: number; widgets: DashboardWidget[] } }];
    expect([app, id, payload.name, payload.version, payload.config.columns]).toEqual(["grants", "d1", "Dean", 3, 12]);
    expect(payload.config.widgets.find((w) => w.id === "chart")?.y).toBe(1);
    expect(payload.config.widgets.find((w) => w.id === "list")?.y).toBe(0);
  });

  it("offers no editing to someone who may not change the dashboard", async () => {
    render(<Dashboard appSlug="grants" view={dashboard()} />);
    await screen.findByText("Ada");
    expect(screen.queryByRole("button", { name: "Edit layout" })).not.toBeInTheDocument();
  });
});

// A page of kind View names a saved view. When that view is a dashboard, the page shows the dashboard.
describe("an app page that names a dashboard", () => {
  const app = (kind: object) =>
    ({
      slug: "grants", title: "Grants", orgCode: "BIO", department: "Biology", customDomain: "", verified: false, hermCapability: "", cedsDomain: "",
      status: "Published", updatedAt: "", recordsCount: 0, workspaceId: "ws-1", collaborators: [],
      manifest: { slug: "grants", title: "Grants", description: "", organization_code: "BIO", department: "Biology", custom_domain_verified: false, tables: [], views: [], ceds_mappings: {}, pages: [{ id: "home", title: "Home", kind }] },
    }) as unknown as import("../types").RegisteredApp;

  it("shows the dashboard on the page", async () => {
    vi.spyOn(apiClient, "listViews").mockResolvedValue({ views: [dashboard()] });
    const { PublishedAppView } = await import("../PublishedAppView");
    render(<PublishedAppView app={app({ View: { view_id: "d1" } })} />);
    expect(await screen.findByRole("region", { name: "Students" })).toBeInTheDocument();
  });
});
