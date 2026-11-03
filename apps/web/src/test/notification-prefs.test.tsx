import "@testing-library/jest-dom";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { NotificationPrefsPanel, NotificationDefaultsEditor } from "../NotificationPrefs";
import { apiClient, type NotificationPrefs } from "../api";

const prefs = (mail = true): NotificationPrefs => ({
  mail_enabled: mail,
  kinds: [
    { kind: "mention", label: "Someone mentions you in a comment", channel: "both", default: "both", chosen: false, forced: false, forced_reason: null },
    { kind: "decision_waiting", label: "A decision is waiting on you", channel: "both", default: "both", chosen: false, forced: true, forced_reason: "A decision waiting on you always reaches your inbox, so nothing stalls unseen." },
  ],
});

// Notifications phase 3, test 4: forced rows cannot be changed and say why.
describe("notification preferences in the settings pane", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("lets a person choose a channel per kind, and shows forced kinds as fixed with the reason", async () => {
    vi.spyOn(apiClient, "getNotificationPrefs").mockResolvedValue(prefs());
    const set = vi.spyOn(apiClient, "setNotificationPref").mockResolvedValue({ kind: "mention", channel: "digest" });
    render(<NotificationPrefsPanel />);
    const mention = (await screen.findByText("Someone mentions you in a comment")).closest("tr")!;
    fireEvent.change(within(mention).getByRole("combobox", { name: /Someone mentions you/ }), { target: { value: "digest" } });
    await waitFor(() => expect(set).toHaveBeenCalledWith("mention", "digest"));

    const forced = screen.getByText("A decision is waiting on you").closest("tr")!;
    expect(within(forced).queryByRole("combobox")).not.toBeInTheDocument();
    expect(within(forced).getByText(/always reaches your inbox, so nothing stalls unseen/)).toBeInTheDocument();
    expect(screen.getByText("Your institution sends mail.")).toBeInTheDocument();
  });

  it("says when the institution sends no mail at all", async () => {
    vi.spyOn(apiClient, "getNotificationPrefs").mockResolvedValue(prefs(false));
    render(<NotificationPrefsPanel />);
    expect(await screen.findByText(/does not send mail/)).toBeInTheDocument();
  });
});

describe("the institution's defaults in the console", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("never offers off for a forced kind, and saves the table as the setting", async () => {
    vi.spyOn(apiClient, "getNotificationPrefs").mockResolvedValue(prefs());
    const save = vi.spyOn(apiClient, "updateSetting").mockResolvedValue({ key: "notifications.defaults", value: {} });
    render(<NotificationDefaultsEditor />);
    const forcedSelect = await screen.findByRole("combobox", { name: /A decision is waiting on you/ });
    expect(within(forcedSelect).queryByRole("option", { name: "Off" })).not.toBeInTheDocument();
    fireEvent.change(screen.getByRole("combobox", { name: /Someone mentions you/ }), { target: { value: "off" } });
    fireEvent.click(screen.getByRole("button", { name: "Save defaults" }));
    await waitFor(() => expect(save).toHaveBeenCalledWith("notifications.defaults", { mention: "off", decision_waiting: "both" }));
  });
});
