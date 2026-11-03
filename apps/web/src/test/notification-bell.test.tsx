import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import { NotificationBell } from "../NotificationBell";
import { apiClient, type InboxNotice } from "../api";
import type { LiveEvent } from "../realtime";

// The person's own stream, faked: a test sends it events.
const live = vi.hoisted(() => ({ listeners: new Set<(e: LiveEvent) => void>() }));
vi.mock("../realtime", async (orig) => ({
  ...(await orig<typeof import("../realtime")>()),
  onMeEvent: (l: (e: LiveEvent) => void) => {
    live.listeners.add(l);
    return () => live.listeners.delete(l);
  },
}));
const emit = (e: LiveEvent) => live.listeners.forEach((l) => l(e));

const notice = (id: string, read: boolean, path = "/app/admissions?instance=i1"): InboxNotice => ({
  id,
  kind: "decision_waiting",
  app_slug: "admissions",
  title: "Decision needed: Admissions",
  body: 'Review "Approve this admission" for Applicants. 3 are waiting.',
  link: { path },
  created_at: "2026-11-02T10:00:00Z",
  read,
});

// Notifications phase 1, test 7: the bell's accessible name includes the count. Opening a notice follows its link and
// marks it read.
describe("the notification bell", () => {
  beforeEach(() => vi.restoreAllMocks());
  afterEach(() => vi.useRealTimers());

  it("says how many are unread in its accessible name", async () => {
    vi.spyOn(apiClient, "notificationCount").mockResolvedValue(3);
    render(<NotificationBell navigate={() => {}} />);
    expect(await screen.findByRole("button", { name: "Notifications, 3 unread" })).toBeInTheDocument();
  });

  it("opens a notice: follows its link and marks it read", async () => {
    vi.spyOn(apiClient, "notificationCount").mockResolvedValue(1);
    vi.spyOn(apiClient, "listNotifications").mockResolvedValue([notice("n1", false), notice("n2", true, "/proposals")]);
    const mark = vi.spyOn(apiClient, "markNotificationsRead").mockResolvedValue(1);
    const navigate = vi.fn();
    render(<NotificationBell navigate={navigate} />);
    fireEvent.click(await screen.findByRole("button", { name: "Notifications, 1 unread" }));
    expect(await screen.findAllByText("Decision needed: Admissions")).toHaveLength(2);
    // Unread is marked with words, not only colour.
    expect(screen.getAllByText("Unread")).toHaveLength(1);
    fireEvent.click(screen.getAllByRole("button", { name: /Decision needed: Admissions/ })[0]);
    await waitFor(() => expect(mark).toHaveBeenCalledWith({ ids: ["n1"] }));
    expect(navigate).toHaveBeenCalledWith("/app/admissions?instance=i1");
  });

  it("marks all read", async () => {
    vi.spyOn(apiClient, "notificationCount").mockResolvedValue(2);
    vi.spyOn(apiClient, "listNotifications").mockResolvedValue([notice("n1", false), notice("n2", false)]);
    const mark = vi.spyOn(apiClient, "markNotificationsRead").mockResolvedValue(2);
    render(<NotificationBell navigate={() => {}} />);
    fireEvent.click(await screen.findByRole("button", { name: "Notifications, 2 unread" }));
    fireEvent.click(await screen.findByRole("button", { name: "Mark all read" }));
    await waitFor(() => expect(mark).toHaveBeenCalledWith({ all: true }));
    expect(await screen.findByRole("button", { name: "Notifications, none unread" })).toBeInTheDocument();
  });

  it("says so when there is nothing", async () => {
    vi.spyOn(apiClient, "notificationCount").mockResolvedValue(0);
    vi.spyOn(apiClient, "listNotifications").mockResolvedValue([]);
    render(<NotificationBell navigate={() => {}} />);
    fireEvent.click(await screen.findByRole("button", { name: "Notifications, none unread" }));
    expect(await screen.findByText("Nothing new")).toBeInTheDocument();
  });

  // Real-time phase 3, test 5: the count follows the person's stream, with no reload and no polling.
  it("updates the count from an event, and no longer polls", async () => {
    vi.useFakeTimers();
    const count = vi.spyOn(apiClient, "notificationCount").mockResolvedValue(0);
    render(<NotificationBell navigate={() => {}} />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByRole("button", { name: "Notifications, none unread" })).toBeInTheDocument();
    act(() => emit({ event: "notification", data: { id: "n9", count: 4 } }));
    expect(screen.getByRole("button", { name: "Notifications, 4 unread" })).toBeInTheDocument();
    const before = count.mock.calls.length;
    await act(async () => {
      vi.advanceTimersByTime(120_000);
      await Promise.resolve();
    });
    expect(count.mock.calls.length).toBe(before);
    // After a reconnect the count is asked for once, since events may have been missed.
    await act(async () => {
      emit({ event: "resync", data: {} });
      await Promise.resolve();
    });
    expect(count.mock.calls.length).toBe(before + 1);
  });
});
