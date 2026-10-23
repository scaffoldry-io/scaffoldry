// MCP apps phase 7: the app's navigation lists the manifest's pages. A View page opens the existing view and a Custom page
// opens the sandboxed frame.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { RegisteredApp } from "../types";

vi.mock("../MultiViewWorkspace", () => ({ MultiViewWorkspace: () => <div data-testid="the-existing-view" /> }));
vi.mock("../PageFrame", () => ({ PageFrame: ({ appSlug, pageId }: { appSlug: string; pageId: string }) => <div data-testid="page-frame-stub">{appSlug}/{pageId}</div> }));

import { PublishedAppView } from "../PublishedAppView";

afterEach(cleanup);

const app = (pages: unknown): RegisteredApp =>
  ({ slug: "scholarships", manifest: { slug: "scholarships", title: "Scholarships", views: [], tables: [], pages } }) as unknown as RegisteredApp;

const PAGES = [
  { id: "list", title: "List", kind: { View: { view_id: "all" } } },
  { id: "board", title: "Award board", kind: { Custom: { source_sha256: "a".repeat(64), grants: [] } } },
];

describe("the published app's pages", () => {
  it("lists the manifest's pages and opens the existing view for a View page", () => {
    render(<PublishedAppView app={app(PAGES)} />);
    expect(screen.getByRole("tab", { name: "List" })).toBeTruthy();
    expect(screen.getByRole("tab", { name: "Award board" })).toBeTruthy();
    expect(screen.getByTestId("the-existing-view")).toBeTruthy();
    expect(screen.queryByTestId("page-frame-stub")).toBeNull();
  });

  it("opens the frame, with this app and page, for a Custom page", () => {
    render(<PublishedAppView app={app(PAGES)} />);
    fireEvent.click(screen.getByRole("tab", { name: "Award board" }));
    expect(screen.getByTestId("page-frame-stub").textContent).toBe("scholarships/board");
    expect(screen.queryByTestId("the-existing-view")).toBeNull();
    fireEvent.click(screen.getByRole("tab", { name: "List" }));
    expect(screen.getByTestId("the-existing-view")).toBeTruthy();
  });

  it("shows no page navigation for an app with no pages, or with only the older builder pages", () => {
    render(<PublishedAppView app={app([])} />);
    expect(screen.queryByRole("tablist")).toBeNull();
    cleanup();
    render(<PublishedAppView app={app([{ id: "p", slug: "p", title: "Builder page", icon: "x", components: [] }])} />);
    expect(screen.queryByRole("tablist")).toBeNull();
    expect(screen.getByTestId("the-existing-view")).toBeTruthy();
  });
});
