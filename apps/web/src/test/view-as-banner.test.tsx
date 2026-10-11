import "@testing-library/jest-dom";
import { describe, it, expect } from "vitest";
import { useState } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { ViewAsBanner } from "../ui";

function Desk() {
  const [viewing, setViewing] = useState(true);
  return viewing ? <ViewAsBanner appTitle="Applicants" person="Dr. Rivera" onExit={() => setViewing(false)} /> : <p>Your own view</p>;
}

describe("ViewAsBanner", () => {
  it("is present, says whose view it is and that it is read-only", () => {
    render(<Desk />);
    expect(screen.getByRole("status")).toHaveTextContent("You are viewing Applicants as Dr. Rivera. This is read-only.");
    expect(screen.getByRole("button", { name: "Exit" })).toBeInTheDocument();
  });

  it("is gone once the person exits", () => {
    render(<Desk />);
    fireEvent.click(screen.getByRole("button", { name: "Exit" }));
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
    expect(screen.getByText("Your own view")).toBeInTheDocument();
  });
});
