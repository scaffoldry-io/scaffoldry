import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/react";
import "@testing-library/jest-dom";
import AdminDesk from "../AdminDesk";
import { MyApps } from "../MyApps";
import { setAuthToken } from "../api";

const APPS = [
  { slug: "alpha", title: "Alpha Register", description: "First", department: "Biology", workspace_id: "ws-1", scope: "app" },
  { slug: "beta", title: "Beta Intake", description: "Second", department: "Physics", workspace_id: "ws-2", scope: "workspace" },
];

describe("MyApps list", () => {
  it("shows pinned apps first, says how each app is reached, and reports pin and open", () => {
    const onTogglePin = vi.fn();
    const onOpen = vi.fn();
    render(<MyApps apps={APPS} pinned={["beta"]} onTogglePin={onTogglePin} onOpen={onOpen} />);
    const pinned = screen.getByTestId("my-apps-pinned");
    expect(within(pinned).getByText("Beta Intake")).toBeInTheDocument();
    expect(within(pinned).queryByText("Alpha Register")).not.toBeInTheDocument();
    expect(screen.getByText(/granted on this app only/i)).toBeInTheDocument();
    expect(screen.getByText(/through a workspace role/i)).toBeInTheDocument();
    fireEvent.click(screen.getByLabelText("Pin Alpha Register"));
    expect(onTogglePin).toHaveBeenCalledWith("alpha");
    fireEvent.click(screen.getByLabelText("Open Alpha Register"));
    expect(onOpen).toHaveBeenCalledWith("alpha");
  });

  it("says so when there are no apps", () => {
    render(<MyApps apps={[]} pinned={[]} onTogglePin={() => {}} onOpen={() => {}} />);
    expect(screen.getByText(/no apps yet/i)).toBeInTheDocument();
  });
});

function stub() {
  global.fetch = vi.fn().mockImplementation((url) => {
    const path = String(url);
    let body: unknown = [];
    if (path.endsWith("/auth/me")) body = { user: { eppn: "gus@state.edu", name: "Gus", affiliation: "faculty", department: "Biology" } };
    if (path.endsWith("/me/apps")) body = { apps: APPS };
    return Promise.resolve({ ok: true, status: 200, statusText: "", json: async () => body });
  }) as unknown as typeof fetch;
}

function openDesk() {
  setAuthToken("tok");
  window.history.pushState({}, "", "/");
  return render(<AdminDesk initialPath="/" />);
}

describe("My apps in the desk", () => {
  beforeEach(() => {
    localStorage.clear();
    localStorage.setItem("scaffoldry_token", "tok");
    stub();
  });

  it("opens from the rail and lists the apps from the server", async () => {
    openDesk();
    fireEvent.click(await screen.findByRole("button", { name: /^my apps/i }));
    expect(await screen.findByTestId("my-apps-view")).toBeInTheDocument();
    expect(await screen.findByText("Alpha Register")).toBeInTheDocument();
    expect(screen.getByText("Beta Intake")).toBeInTheDocument();
  });

  it("keeps a pinned app in the rail, and keeps it after a reload", async () => {
    const first = openDesk();
    fireEvent.click(await screen.findByRole("button", { name: /^my apps/i }));
    fireEvent.click(await screen.findByLabelText("Pin Alpha Register"));
    const group = await screen.findByTestId("pinned-apps-group");
    expect(within(group).getByText("Alpha Register")).toBeInTheDocument();
    expect(JSON.parse(localStorage.getItem("scaffoldry_pinned_apps") || "[]")).toEqual(["alpha"]);
    first.unmount();

    openDesk();
    const again = await screen.findByTestId("pinned-apps-group");
    expect(within(again).getByText("Alpha Register")).toBeInTheDocument();
  });

  it("does not open some other app when the desk does not have the one asked for", async () => {
    setAuthToken("tok");
    window.history.pushState({}, "", "/app/an-app-the-desk-does-not-have");
    render(<AdminDesk initialPath="/app/an-app-the-desk-does-not-have" />);
    expect(await screen.findByTestId("app-not-in-desk")).toBeInTheDocument();
    expect(screen.queryByTestId("published-app-runtime")).not.toBeInTheDocument();
    fireEvent.click(screen.getByText("Back to My apps"));
    expect(await screen.findByTestId("my-apps-view")).toBeInTheDocument();
  });
});
