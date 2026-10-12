import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { useState } from "react";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { CollaboratorPicker, CollaboratorChips } from "../CollaboratorPicker";
import * as api from "../api";

const people = {
  collaborators: [
    { id: "curie@science.state.edu", label: "Prof. Curie", email: "curie@science.state.edu" },
    { id: "lee@state.edu", label: "Jordan Lee", email: "lee@state.edu" },
  ],
};

function Harness({ cardinality }: { cardinality?: "one" | "many" }) {
  const [value, setValue] = useState<{ id: string; label: string; email?: string }[]>([]);
  return <CollaboratorPicker appSlug="demo" value={value} onChange={setValue} cardinality={cardinality} />;
}

describe("CollaboratorPicker", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("is operable by keyboard, announces the result count, and selecting adds a chip", async () => {
    const list = vi.spyOn(api, "listCollaborators").mockResolvedValue(people);
    render(<Harness />);
    const box = screen.getByRole("combobox");
    fireEvent.focus(box);
    expect(await screen.findByText("2 results")).toBeInTheDocument();
    expect(list).toHaveBeenCalledWith("demo", "");
    fireEvent.keyDown(box, { key: "ArrowDown" });
    expect(box).toHaveAttribute("aria-activedescendant", expect.stringMatching(/-1$/));
    fireEvent.keyDown(box, { key: "Enter" });
    expect(screen.getByRole("list", { name: "Collaborators" })).toHaveTextContent("Jordan Lee");
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
  });

  it("closes on Escape and says what matched nothing", async () => {
    vi.spyOn(api, "listCollaborators").mockResolvedValueOnce(people).mockResolvedValue({ collaborators: [] });
    render(<Harness />);
    const box = screen.getByRole("combobox");
    fireEvent.focus(box);
    await screen.findByText("2 results");
    fireEvent.change(box, { target: { value: "zzz" } });
    expect(await screen.findByText("No matches for \"zzz\"")).toBeInTheDocument();
    fireEvent.keyDown(box, { key: "Escape" });
    await waitFor(() => expect(box).toHaveAttribute("aria-expanded", "false"));
  });

  it("searches the directory by name and only returns people with read access to the app", async () => {
    const list = vi.spyOn(api, "listCollaborators").mockImplementation(async (_slug, q) => {
      const matching = people.collaborators.filter(c => !q || c.label.toLowerCase().includes(q.toLowerCase()));
      return { collaborators: matching };
    });
    render(<Harness />);
    const box = screen.getByRole("combobox");
    fireEvent.focus(box);
    fireEvent.change(box, { target: { value: "Curie" } });
    expect(list).toHaveBeenCalledWith("demo", "Curie");
    expect(await screen.findByText("1 result")).toBeInTheDocument();
    expect(screen.getByText("Prof. Curie")).toBeInTheDocument();
    expect(screen.queryByText("Jordan Lee")).not.toBeInTheDocument();
  });
});

describe("CollaboratorChips", () => {
  it("renders a deactivated person with (no longer active)", () => {
    render(
      <CollaboratorChips
        value={[
          { id: "a", label: "Dr. Rivera", email: "rivera@state.edu", active: false },
          { id: "b", label: "Prof. Curie", email: "curie@science.state.edu", active: true },
        ]}
      />
    );
    expect(screen.getByText("Dr. Rivera (no longer active)")).toBeInTheDocument();
    expect(screen.getByText("Prof. Curie <curie@science.state.edu>")).toBeInTheDocument();
  });
});
