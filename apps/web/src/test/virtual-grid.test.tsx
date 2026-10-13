import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import { VirtualGrid, rowCountText } from "../ui";

const TOTAL = 1_000_000;
const ROW = 32;
const VIEWPORT = 320;
// The grid keeps its scroll area to 10 million pixels, so a million rows of 32 scroll 3.2 times faster than their pixels.
const RATIO = (TOTAL * ROW) / 10_000_000;

interface Row {
  amount: number;
}

const columns = [{ key: "amount", header: "Amount" }];

/** The rows the server would send for a window: the amount is the row number plus one. */
const windowOf = (offset: number, limit: number): Row[] => Array.from({ length: Math.min(limit, TOTAL - offset) }, (_, i) => ({ amount: offset + i + 1 }));

// jsdom has no layout, so the viewport gets a size and a scrollTop that stick.
beforeEach(() => {
  Object.defineProperty(HTMLElement.prototype, "offsetHeight", { configurable: true, get: () => VIEWPORT });
  Object.defineProperty(HTMLElement.prototype, "offsetWidth", { configurable: true, get: () => 800 });
  HTMLElement.prototype.getBoundingClientRect = () => ({ width: 800, height: VIEWPORT, top: 0, left: 0, right: 800, bottom: VIEWPORT, x: 0, y: 0, toJSON: () => ({}) });
  HTMLElement.prototype.scrollTo = function (this: HTMLElement, opts?: ScrollToOptions | number) {
    const top = typeof opts === "number" ? opts : (opts?.top ?? 0);
    Object.defineProperty(this, "scrollTop", { configurable: true, writable: true, value: top });
    this.dispatchEvent(new Event("scroll"));
  } as any;
});

afterEach(() => {
  delete (HTMLElement.prototype as any).scrollTo;
});

const grid = () => screen.getByRole("grid");
const rowAt = (index: number) => screen.getAllByRole("row").find((r) => r.getAttribute("aria-rowindex") === String(index));
const bodyRows = () => screen.getAllByRole("row").filter((r) => r.getAttribute("aria-rowindex") !== "1");

function setup(fetchWindow = vi.fn(async (offset: number, limit: number) => windowOf(offset, limit))) {
  render(<VirtualGrid label="Amounts" columns={columns} total={TOTAL} fetchWindow={fetchWindow} rowHeight={ROW} viewportHeight={VIEWPORT} overscan={5} />);
  return fetchWindow;
}

describe("VirtualGrid — a million rows in a window", () => {
  it("renders only the visible rows and a buffer, with the right row numbers", async () => {
    setup();
    await waitFor(() => expect(screen.getByText("1")).toBeInTheDocument());
    expect(grid()).toHaveAttribute("aria-rowcount", String(TOTAL + 1));
    const rows = bodyRows();
    // Ten fit the viewport, and the buffer adds five below. Nothing like a million.
    expect(rows.length).toBeLessThanOrEqual(VIEWPORT / ROW + 5 * 2);
    expect(rows[0]).toHaveAttribute("aria-rowindex", "2");
    expect(rows[1]).toHaveAttribute("aria-rowindex", "3");
    // The header is row one.
    expect(screen.getAllByRole("row").find((r) => r.getAttribute("aria-rowindex") === "1")).toBeDefined();
  });

  it("fetches one window of 200 for the rows in view, and no other", async () => {
    const fetchWindow = setup();
    await waitFor(() => expect(fetchWindow).toHaveBeenCalled());
    expect(fetchWindow).toHaveBeenCalledTimes(1);
    expect(fetchWindow).toHaveBeenCalledWith(0, 200);
  });

  it("shows skeleton rows while a window loads, and the rows after", async () => {
    let release: (rows: Row[]) => void = () => {};
    const pending = new Promise<Row[]>((resolve) => (release = resolve));
    setup(vi.fn(() => pending));
    await waitFor(() => expect(bodyRows().length).toBeGreaterThan(0));
    expect(bodyRows().every((r) => r.getAttribute("aria-busy") === "true")).toBe(true);
    expect(screen.queryByText("1")).toBeNull();
    await act(async () => release(windowOf(0, 200)));
    await waitFor(() => expect(screen.getByText("1")).toBeInTheDocument());
    expect(bodyRows()[0]).not.toHaveAttribute("aria-busy", "true");
  });

  it("loads the end with Ctrl+End, and the last row is numbered after the header and every record", async () => {
    const fetchWindow = setup();
    await waitFor(() => expect(fetchWindow).toHaveBeenCalledWith(0, 200));
    fireEvent.keyDown(grid(), { key: "End", ctrlKey: true });
    await waitFor(() => expect(fetchWindow).toHaveBeenCalledWith(999_800, 200));
    await waitFor(() => expect(screen.getByText("1000000")).toBeInTheDocument());
    const last = bodyRows().at(-1)!;
    expect(last).toHaveAttribute("aria-rowindex", String(TOTAL + 1));
    expect(last).toHaveTextContent("1000000");
  });

  it("returns to the front with Ctrl+Home, and pages with Page Down and Page Up", async () => {
    const fetchWindow = setup();
    await waitFor(() => expect(fetchWindow).toHaveBeenCalledWith(0, 200));
    const viewport = () => grid().querySelector("[data-viewport]") as HTMLElement;
    fireEvent.keyDown(grid(), { key: "PageDown" });
    await waitFor(() => expect(viewport()).toHaveAttribute("data-offset", String(VIEWPORT)));
    expect(rowAt(12)).toBeDefined();
    fireEvent.keyDown(grid(), { key: "PageUp" });
    await waitFor(() => expect(viewport()).toHaveAttribute("data-offset", "0"));
    fireEvent.keyDown(grid(), { key: "End", ctrlKey: true });
    await waitFor(() => expect(screen.getByText("1000000")).toBeInTheDocument());
    fireEvent.keyDown(grid(), { key: "Home", ctrlKey: true });
    await waitFor(() => expect(viewport()).toHaveAttribute("data-offset", "0"));
    await waitFor(() => expect(screen.getByText("1")).toBeInTheDocument());
    expect(bodyRows()[0]).toHaveAttribute("aria-rowindex", "2");
  });

  it("loads the window under the scrollbar when it is dragged to the middle", async () => {
    const fetchWindow = setup();
    await waitFor(() => expect(fetchWindow).toHaveBeenCalledWith(0, 200));
    const viewport = grid().querySelector("[data-viewport]") as HTMLElement;
    await act(async () => {
      (viewport as any).scrollTop = (500_000 * ROW) / RATIO;
      viewport.dispatchEvent(new Event("scroll"));
    });
    await waitFor(() => expect(fetchWindow).toHaveBeenCalledWith(500_000, 200));
    await waitFor(() => expect(screen.getByText("500001")).toBeInTheDocument());
    expect(rowAt(500_000 + 2)).toHaveTextContent("500001");
  });
});

describe("rowCountText", () => {
  it("says the count, or that it is over the most that could be counted", () => {
    expect(rowCountText({ value: 1_000_000, exact: true })).toBe("1,000,000 records");
    expect(rowCountText({ value: 1, exact: true })).toBe("1 record");
    expect(rowCountText({ at_least: 250_000 })).toBe("Over 250,000 records");
    expect(rowCountText({ at_least: 0 })).toBe("Too many records to count");
  });
});
