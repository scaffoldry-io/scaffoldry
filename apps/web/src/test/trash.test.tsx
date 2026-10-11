import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { Trash } from "../Trash";
import { UndoDeleteToast } from "../UndoDeleteToast";
import * as api from "../api";

const entry: api.TrashEntry = {
  record_id: "r-1",
  table_id: "t",
  title: "Ada",
  deleted_by: "a@x.edu",
  deleted_by_name: "Ann",
  deleted_at: "2026-10-11T03:00:00Z",
  delete_batch: "b-1",
  days_left: 27,
};

describe("trash", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("Restore in trash restores the record and reloads the list", async () => {
    const list = vi.spyOn(api, "listTrash").mockResolvedValueOnce({ entries: [entry], next_cursor: null }).mockResolvedValue({ entries: [], next_cursor: null });
    const restore = vi.spyOn(api, "restoreTrash").mockResolvedValue({ restored: 1, batch_id: "b-2" });
    render(<Trash appSlug="demo" />);
    expect(await screen.findByText("Removed permanently in 27 days")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Restore Ada" }));
    await waitFor(() => expect(restore).toHaveBeenCalledWith("demo", "r-1"));
    expect(await screen.findByText("The trash is empty")).toBeInTheDocument();
    expect(list).toHaveBeenCalledTimes(2);
  });

  it("Undo on the toast restores the delete batch", async () => {
    const restore = vi.spyOn(api, "restoreTrash").mockResolvedValue({ restored: 2, batch_id: "b-2" });
    const closed = vi.fn();
    const restored = vi.fn();
    render(<UndoDeleteToast appSlug="demo" deleteBatch="b-1" onClose={closed} onRestored={restored} />);
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    await waitFor(() => expect(restore).toHaveBeenCalledWith("demo", "b-1"));
    await waitFor(() => expect(restored).toHaveBeenCalled());
    expect(closed).toHaveBeenCalled();
  });
});
