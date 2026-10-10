import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { Positions } from "../admin/Positions";
import { UnitPositions } from "../admin/UnitPositions";
import { apiClient } from "../api";

const chair = { key: "chair", name: "Department Chair", description: "", org_types: ["Department"], max_holders: 1, retired: false, holder_count: 0 };

describe("Admin Positions", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.spyOn(apiClient, "adminListPositions").mockResolvedValue({ positions: [chair] });
    vi.spyOn(apiClient, "adminPositionVacancies").mockResolvedValue({
      vacancies: [{ unit: { id: "u-1", name: "Physics" }, org_type: "Department", position: { key: "chair", name: "Department Chair" } }],
    });
  });

  it("lists position types and shows vacancies with an Assign action", async () => {
    render(<Positions />);
    expect(await screen.findByText("Department Chair")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("tab", { name: /vacancies/i }));
    expect(await screen.findByText("Physics")).toBeInTheDocument();
    expect(screen.getByTestId("vacancy-assign-u-1-chair")).toBeInTheDocument();
  });

  it("Assign from a vacancy needs a reason, then sends the holder", async () => {
    const assign = vi.spyOn(apiClient, "assignPositionHolder").mockResolvedValue({});
    render(<Positions />);
    fireEvent.click(await screen.findByRole("tab", { name: /vacancies/i }));
    fireEvent.click(await screen.findByTestId("vacancy-assign-u-1-chair"));
    fireEvent.change(await screen.findByLabelText(/person/i), { target: { value: "rivera@state.edu" } });
    const confirm = await screen.findByRole("button", { name: /confirm assignment/i });
    expect(confirm).toBeDisabled();
    fireEvent.change(screen.getByLabelText(/reason for decision/i), { target: { value: "New chair" } });
    fireEvent.click(confirm);
    await waitFor(() =>
      expect(assign).toHaveBeenCalledWith("u-1", "chair", { eppn: "rivera@state.edu", replace: false, reason: "New chair" })
    );
  });

  it("the create form posts a new position type", async () => {
    const create = vi.spyOn(apiClient, "adminCreatePosition").mockResolvedValue({ position: chair });
    render(<Positions />);
    fireEvent.click(await screen.findByTestId("position-create-open"));
    const form = await screen.findByTestId("position-create-form");
    fireEvent.change(form.querySelector("#position-key")!, { target: { value: "dean" } });
    fireEvent.change(form.querySelector("#position-name")!, { target: { value: "Dean" } });
    fireEvent.click(screen.getByLabelText("College"));
    fireEvent.change(form.querySelector("#position-reason")!, { target: { value: "Colleges need deans" } });
    fireEvent.click(screen.getByRole("button", { name: /create position type/i }));
    await waitFor(() => expect(create).toHaveBeenCalled());
    expect(create.mock.calls[0][0]).toMatchObject({ key: "dean", org_types: ["College"], reason: "Colleges need deans" });
  });
});

describe("Unit positions", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("shows Vacant for an empty position", async () => {
    vi.spyOn(apiClient, "listUnitPositions").mockResolvedValue({
      positions: [{ key: "chair", name: "Department Chair", description: "", max_holders: 1, holders: [] }],
    });
    render(<UnitPositions unitId="u-1" />);
    expect(await screen.findByText("Vacant")).toBeInTheDocument();
    expect(screen.getByTestId("position-assign-chair")).toBeInTheDocument();
  });

  it("Vacate without a reason is not sent", async () => {
    vi.spyOn(apiClient, "listUnitPositions").mockResolvedValue({
      positions: [
        { key: "chair", name: "Department Chair", description: "", max_holders: 1, holders: [{ eppn: "rivera@state.edu", display_name: "Dr. Rivera", source: "api" }] },
      ],
    });
    const vacate = vi.spyOn(apiClient, "vacatePositionHolder").mockResolvedValue({});
    render(<UnitPositions unitId="u-1" />);
    fireEvent.click(await screen.findByTestId("position-vacate-chair-rivera@state.edu"));
    const confirm = await screen.findByRole("button", { name: /confirm vacate/i });
    expect(confirm).toBeDisabled();
    fireEvent.click(confirm);
    expect(vacate).not.toHaveBeenCalled();

    fireEvent.change(screen.getByLabelText(/reason for decision/i), { target: { value: "Stepped down" } });
    fireEvent.click(confirm);
    await waitFor(() => expect(vacate).toHaveBeenCalledWith("u-1", "chair", "rivera@state.edu", "Stepped down"));
  });

  it("a registry holding cannot be vacated from the screen", async () => {
    vi.spyOn(apiClient, "listUnitPositions").mockResolvedValue({
      positions: [
        { key: "chair", name: "Department Chair", description: "", max_holders: 1, holders: [{ eppn: "okafor@state.edu", display_name: "Dr. Okafor", source: "scim" }] },
      ],
    });
    render(<UnitPositions unitId="u-1" />);
    expect(await screen.findByText(/set by the registry/i)).toBeInTheDocument();
    expect(screen.queryByTestId("position-vacate-chair-okafor@state.edu")).not.toBeInTheDocument();
  });
});
