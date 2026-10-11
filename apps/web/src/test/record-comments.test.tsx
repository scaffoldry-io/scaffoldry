import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { RecordComments } from "../RecordComments";
import { apiClient, type CommentRow } from "../api";

const people = [
  { eppn: "student.smith@science.state.edu", name: "Alex Smith" },
  { eppn: "marcus.vance@state.edu", name: "Marcus Vance" },
];

const live: CommentRow = {
  id: "c-1",
  record_id: "r-1",
  parent_id: null,
  author: "prof.curie@science.state.edu",
  author_name: "Dr. Marie Curie",
  body: "Please check the total",
  mentions: [],
  deleted: false,
  created_at: "2026-10-11T03:00:00.000Z",
  edited_at: null,
  deleted_at: null,
};

const removed: CommentRow = {
  ...live,
  id: "c-2",
  author: "student.smith@science.state.edu",
  author_name: "Alex Smith",
  body: null,
  deleted: true,
  created_at: "2026-10-11T03:05:00.000Z",
  deleted_at: "2026-10-11T03:06:00.000Z",
};

describe("RecordComments", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.spyOn(apiClient, "listComments").mockResolvedValue({ comments: [live, removed] });
    vi.spyOn(apiClient, "mentionable").mockResolvedValue({ people });
  });

  it("the picker lists only the people the server says can read the app, and a pick writes the address", async () => {
    const post = vi.spyOn(apiClient, "postComment").mockResolvedValue({ ...live, id: "c-3" });
    render(<RecordComments appSlug="lab" recordId="r-1" viewerEppn="prof.curie@science.state.edu" />);
    const box = await screen.findByLabelText("Write a comment");
    fireEvent.change(box, { target: { value: "Thanks @" } });

    const options = await screen.findAllByRole("option");
    expect(options.map((o) => o.textContent)).toEqual(["Alex Smith", "Marcus Vance"]);
    fireEvent.click(screen.getByRole("option", { name: "Alex Smith" }));
    expect((box as HTMLTextAreaElement).value).toBe("Thanks @[student.smith@science.state.edu] ");

    fireEvent.click(screen.getByRole("button", { name: "Comment" }));
    await waitFor(() => expect(post).toHaveBeenCalledWith("lab", "r-1", "Thanks @[student.smith@science.state.edu] "));
  });

  it("typing after the @ narrows the picker", async () => {
    render(<RecordComments appSlug="lab" recordId="r-1" viewerEppn="prof.curie@science.state.edu" />);
    const box = await screen.findByLabelText("Write a comment");
    fireEvent.change(box, { target: { value: "@mar" } });
    const options = await screen.findAllByRole("option");
    expect(options.map((o) => o.textContent)).toEqual(["Marcus Vance"]);
  });

  it("a removed comment shows the placeholder, with its author and time, and no text", async () => {
    render(<RecordComments appSlug="lab" recordId="r-1" viewerEppn="prof.curie@science.state.edu" />);
    expect(await screen.findByText("Please check the total")).toBeInTheDocument();
    const placeholder = screen.getByText("Comment removed");
    expect(placeholder).toBeInTheDocument();
    const item = placeholder.closest("li");
    expect(item).toHaveTextContent("Alex Smith");
    expect(item).toHaveTextContent("2026-10-11 03:05");
  });
});
