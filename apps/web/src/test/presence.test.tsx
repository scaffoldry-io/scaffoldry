import "@testing-library/jest-dom";
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { PresenceAvatars } from "../ui/PresenceAvatars";
import { presenceAfter, type Present } from "../realtime";

const person = (n: number): Present => ({ address: `p${n}@state.edu`, name: `Person ${n}` });

// Real-time phase 3, test 5: the header shows up to five avatars with names, and `+3` for the rest.
describe("who else has the app open", () => {
  it("shows up to five avatars, each with a name in words, and a count for the rest", () => {
    render(<PresenceAvatars present={[1, 2, 3, 4, 5, 6, 7, 8].map(person)} />);
    const list = screen.getByRole("list", { name: "8 people have this app open" });
    expect(list).toBeInTheDocument();
    for (const n of [1, 2, 3, 4, 5]) expect(screen.getByText(`Person ${n}`)).toBeInTheDocument();
    expect(screen.queryByText("Person 6")).not.toBeInTheDocument();
    expect(screen.getByText("+3")).toBeInTheDocument();
    expect(screen.getByLabelText("and 3 more")).toBeInTheDocument();
  });

  it("shows nothing when nobody else is here", () => {
    const { container } = render(<PresenceAvatars present={[]} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("follows the list on connect, then joins and leaves", () => {
    let state = presenceAfter({ present: [] }, { event: "presence", data: { present: [person(1)], count: 1, stream: "s1" } });
    expect(state).toEqual({ present: [person(1)], stream: "s1" });
    state = presenceAfter(state, { event: "presence", data: { joined: person(2), count: 2 } });
    expect(state.present.map((p) => p.address)).toEqual(["p1@state.edu", "p2@state.edu"]);
    state = presenceAfter(state, { event: "presence", data: { joined: person(2), count: 2 } });
    expect(state.present).toHaveLength(2);
    state = presenceAfter(state, { event: "presence", data: { left: person(1), count: 1 } });
    expect(state.present.map((p) => p.address)).toEqual(["p2@state.edu"]);
    expect(state.stream).toBe("s1");
    // Other events change nothing.
    expect(presenceAfter(state, { event: "change", data: {} })).toBe(state);
  });
});
