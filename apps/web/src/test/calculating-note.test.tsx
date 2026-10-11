import "@testing-library/jest-dom";
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { CalculatingNote } from "../ui";

describe("CalculatingNote", () => {
  it("says how far along the job is", () => {
    render(<CalculatingNote percent={63} />);
    expect(screen.getByRole("status")).toHaveTextContent("Calculating, 63% done");
  });

  it("says only that it is calculating when it does not know", () => {
    render(<CalculatingNote />);
    expect(screen.getByRole("status")).toHaveTextContent(/^Calculating$/);
  });
});
