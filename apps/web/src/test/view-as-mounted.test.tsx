import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent, within } from "@testing-library/react";
import "@testing-library/jest-dom";
import AdminDesk from "../AdminDesk";
import { getAuthToken, setAuthToken } from "../api";

const APP = "Biology Lab Equipment & Bioassay Register";
const BANNER = `You are viewing ${APP} as Alex Smith. This is read-only.`;

type Reply = { status?: number; body: unknown };

function stub(viewAs: Reply = { body: { token: "viewas-token", expires_at: "2026-10-11T23:00:00Z", app_slug: "bio-lab-inventory", target: { eppn: "student.smith@science.state.edu", name: "Alex Smith" }, read_only: true } }) {
  const fetchMock = vi.fn().mockImplementation((url, options) => {
    const path = String(url);
    let reply: Reply = { body: [] };
    if (path.endsWith("/apps/bio-lab-inventory/view-as")) reply = viewAs;
    if (path.endsWith("/auth/stop-view-as")) reply = { body: { status: "ended" } };
    const status = reply.status ?? 200;
    return Promise.resolve({ ok: status < 400, status, statusText: "", json: async () => reply.body });
  });
  global.fetch = fetchMock;
  return fetchMock;
}

const authOf = ([, options]: [unknown, RequestInit]) => (options?.headers as Record<string, string>)?.Authorization;
const calls = (fetchMock: ReturnType<typeof vi.fn>, suffix: string) => fetchMock.mock.calls.filter(([u]) => String(u).endsWith(suffix));

async function openCollaborators(eppn: string) {
  localStorage.clear();
  localStorage.setItem("scaffoldry_token", "own-token");
  setAuthToken("own-token");
  window.history.pushState({}, "", "/workspace/ws-bio-lab");
  render(<AdminDesk initialPath="/" initialEppn={eppn} />);
  const title = (await screen.findAllByText(APP))[0];
  let card: HTMLElement | null = title as HTMLElement;
  while (card && !card.querySelector('[title="Quick Tabular Grid & Views"]')) card = card.parentElement;
  fireEvent.click(card!.querySelector('[title="Quick Tabular Grid & Views"]')!);
  fireEvent.click(await screen.findByText(/Configure in Studio/));
  fireEvent.click(await screen.findByText(/Team Collaborators/));
  await screen.findByText(/Co-Building Team/);
}

describe("View as, on the desk", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("an app owner can view as another collaborator, but not as themselves", async () => {
    stub();
    await openCollaborators("prof.curie@science.state.edu");
    expect(screen.getAllByRole("button", { name: "View as" })).toHaveLength(1);
    const row = screen.getByText("Alex Smith").closest("div.flex.items-center.justify-between") as HTMLElement;
    expect(within(row).getByRole("button", { name: "View as" })).toBeInTheDocument();
  });

  it("starting a view shows the banner, and every request after it carries the view-as token", async () => {
    const fetchMock = stub();
    await openCollaborators("prof.curie@science.state.edu");
    fireEvent.click(screen.getByRole("button", { name: "View as" }));

    expect(await screen.findByText(BANNER, { exact: false })).toBeInTheDocument();
    const [, startOptions] = calls(fetchMock, "/apps/bio-lab-inventory/view-as")[0];
    expect(JSON.parse(startOptions.body)).toEqual({ eppn: "student.smith@science.state.edu" });
    expect(authOf(calls(fetchMock, "/apps/bio-lab-inventory/view-as")[0])).toBe("Bearer own-token");

    // Every request from now on carries the view-as token.
    expect(getAuthToken()).toBe("viewas-token");
    // A reload must not keep the view without its banner, so the view-as token is never stored.
    expect(localStorage.getItem("scaffoldry_token")).toBe("own-token");
  });

  it("exit revokes the view-as token, takes the banner away and restores the person's own token", async () => {
    const fetchMock = stub();
    await openCollaborators("prof.curie@science.state.edu");
    fireEvent.click(screen.getByRole("button", { name: "View as" }));
    await screen.findByText(BANNER, { exact: false });

    fireEvent.click(screen.getByRole("button", { name: "Exit" }));
    await waitFor(() => expect(screen.queryByText(BANNER, { exact: false })).not.toBeInTheDocument());

    const stops = calls(fetchMock, "/auth/stop-view-as");
    expect(stops).toHaveLength(1);
    expect(authOf(stops[0])).toBe("Bearer viewas-token");
    // The next request is the person's own again.
    expect(getAuthToken()).toBe("own-token");
    expect(localStorage.getItem("scaffoldry_token")).toBe("own-token");
  });

  it("an editor is not offered View as", async () => {
    stub();
    await openCollaborators("student.smith@science.state.edu");
    expect(screen.queryByRole("button", { name: "View as" })).not.toBeInTheDocument();
  });

  it("a refused view shows the server's reason, no banner, and keeps the person's own token", async () => {
    const fetchMock = stub({ status: 403, body: { error: "Only an app owner or admin can view as another person." } });
    await openCollaborators("prof.curie@science.state.edu");
    fireEvent.click(screen.getByRole("button", { name: "View as" }));

    expect(await screen.findByText("Only an app owner or admin can view as another person.")).toBeInTheDocument();
    expect(screen.queryByText(BANNER, { exact: false })).not.toBeInTheDocument();
    expect(fetchMock).toHaveBeenCalled();
    expect(getAuthToken()).toBe("own-token");
  });
});
