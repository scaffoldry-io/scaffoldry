import { useCallback, useEffect, useState } from "react";
import { apiClient, type InboxNotice } from "./api";
import { EmptyState, ErrorState } from "./ui";

/** How often the bell checks the count while there is no live channel for it. */
const POLL_MS = 60_000;

function countName(n: number): string {
  return n === 0 ? "Notifications, none unread" : `Notifications, ${n} unread`;
}

function when(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString("en-US", { dateStyle: "medium", timeStyle: "short" });
}

// The bell in the top bar (notifications phase 1). Its accessible name says how many notices are unread. Opening it
// lists them, newest first. Opening a notice follows its link and marks it read. A notice holds names and links only.
export function NotificationBell({ navigate }: { navigate: (path: string) => void }) {
  const [unread, setUnread] = useState(0);
  const [open, setOpen] = useState(false);
  const [items, setItems] = useState<InboxNotice[] | null>(null);
  const [error, setError] = useState<unknown>(null);

  const refreshCount = useCallback(async () => {
    try {
      setUnread(await apiClient.notificationCount());
    } catch {
      // The count is a convenience. A failed check leaves the last one shown.
    }
  }, []);

  useEffect(() => {
    void refreshCount();
    const timer = setInterval(() => void refreshCount(), POLL_MS);
    return () => clearInterval(timer);
  }, [refreshCount]);

  const load = useCallback(async () => {
    setError(null);
    try {
      setItems(await apiClient.listNotifications());
    } catch (e) {
      setError(e);
      setItems([]);
    }
  }, []);

  const toggle = () => {
    const next = !open;
    setOpen(next);
    if (next) void load();
  };

  const openNotice = async (n: InboxNotice) => {
    if (!n.read) {
      try {
        await apiClient.markNotificationsRead({ ids: [n.id] });
        setUnread((u) => Math.max(0, u - 1));
        setItems((list) => (list ?? []).map((x) => (x.id === n.id ? { ...x, read: true } : x)));
      } catch {
        // Following the link still matters more than the mark.
      }
    }
    setOpen(false);
    if (n.link?.path) navigate(n.link.path);
  };

  const markAll = async () => {
    try {
      await apiClient.markNotificationsRead({ all: true });
      setUnread(0);
      setItems((list) => (list ?? []).map((x) => ({ ...x, read: true })));
    } catch (e) {
      setError(e);
    }
  };

  return (
    <div className="relative">
      <button
        type="button"
        aria-label={countName(unread)}
        aria-expanded={open}
        onClick={toggle}
        className="relative p-1.5 rounded-md text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-800 cursor-pointer"
      >
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9" />
        </svg>
        {unread > 0 && (
          <span aria-hidden="true" className="absolute -top-0.5 -right-0.5 min-w-4 h-4 px-1 rounded-full bg-rose-600 text-white text-[10px] font-bold leading-4 text-center">
            {unread > 99 ? "99+" : unread}
          </span>
        )}
      </button>
      {open && (
        <div role="dialog" aria-label="Notifications" className="absolute right-0 mt-2 w-80 max-h-[28rem] overflow-auto rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 shadow-lg z-50">
          <div className="flex items-center justify-between px-3 py-2 border-b border-slate-100 dark:border-slate-800">
            <span className="text-sm font-semibold">Notifications</span>
            <button type="button" className="text-xs underline disabled:opacity-50" disabled={(items ?? []).every((n) => n.read)} onClick={() => void markAll()}>
              Mark all read
            </button>
          </div>
          {error !== null && <ErrorState error={error} />}
          {items === null ? (
            <p className="p-3 text-xs text-slate-500">Loading…</p>
          ) : items.length === 0 ? (
            <EmptyState title="Nothing new" description="Decisions, mentions and finished work show up here." />
          ) : (
            <ul>
              {items.map((n) => (
                <li key={n.id} className="border-b border-slate-100 dark:border-slate-800 last:border-0">
                  <button type="button" onClick={() => void openNotice(n)} className={`w-full text-left px-3 py-2 hover:bg-slate-50 dark:hover:bg-slate-800/60 ${n.read ? "" : "bg-blue-50/60 dark:bg-blue-950/30"}`}>
                    <span className="block text-sm font-medium">{n.title}</span>
                    <span className="block text-xs text-slate-600 dark:text-slate-400">{n.body}</span>
                    <span className="block text-[11px] text-slate-500 mt-0.5">
                      {!n.read && <span className="font-semibold text-blue-700 dark:text-blue-300 mr-1">Unread</span>}
                      {when(n.created_at)}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}
