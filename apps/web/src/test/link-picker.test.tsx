import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { useState } from "react";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { LinkPicker, LinkChips } from "../LinkPicker";
import * as api from "../api";

const page = { options: [{ id: "p1", label: "CS", secondary: "Computer Science" }, { id: "p2", label: "BIO", secondary: null }], next_cursor: null };

function Harness() {
  const [value, setValue] = useState<{ id: string; label: string }[]>([]);
  return <LinkPicker appSlug="demo" tableId="programs" value={value} onChange={setValue} />;
}

describe("LinkPicker", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("is operable by keyboard, announces the result count, and selecting adds a chip", async () => {
    const list = vi.spyOn(api, "listLinkOptions").mockResolvedValue(page);
    render(<Harness />);
    const box = screen.getByRole("combobox");
    fireEvent.focus(box);
    expect(await screen.findByText("2 results")).toBeInTheDocument();
    expect(list).toHaveBeenCalledWith("demo", "programs", "");
    fireEvent.keyDown(box, { key: "ArrowDown" });
    expect(box).toHaveAttribute("aria-activedescendant", expect.stringMatching(/-1$/));
    fireEvent.keyDown(box, { key: "Enter" });
    expect(screen.getByRole("list", { name: "Linked records" })).toHaveTextContent("BIO");
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
  });

  it("closes on Escape and says what matched nothing", async () => {
    vi.spyOn(api, "listLinkOptions").mockResolvedValueOnce(page).mockResolvedValue({ options: [], next_cursor: null });
    render(<Harness />);
    const box = screen.getByRole("combobox");
    fireEvent.focus(box);
    await screen.findByText("2 results");
    fireEvent.change(box, { target: { value: "zzz" } });
    expect(await screen.findByText('No matches for "zzz"')).toBeInTheDocument();
    fireEvent.keyDown(box, { key: "Escape" });
    await waitFor(() => expect(box).toHaveAttribute("aria-expanded", "false"));
  });
});

describe("LinkChips", () => {
  it("makes a chip a link only when the person can open the record", () => {
    const onOpen = vi.fn();
    render(<LinkChips value={[{ id: "a", label: "Open one" }, { id: "b", label: "Closed one" }]} canOpen={(id) => id === "a"} onOpen={onOpen} />);
    fireEvent.click(screen.getByRole("button", { name: "Open one" }));
    expect(onOpen).toHaveBeenCalledWith("a");
    expect(screen.getByText("Closed one")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Closed one" })).not.toBeInTheDocument();
  });
});
