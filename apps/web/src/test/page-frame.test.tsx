// MCP apps phase 7: the desk hosts a custom page in a sandboxed frame and is the only thing that talks to the server.
import { render, screen, waitFor, cleanup, act } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PageFrame } from "../PageFrame";
import { setAuthToken } from "../api";

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });

let fetchMock: ReturnType<typeof vi.fn>;
const originalFetch = global.fetch;

beforeEach(() => {
  setAuthToken("desk-token");
  fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
    if (init?.method === "POST") return json({ records: [{ id: "r1", version: 1, data: { title: "A" } }], next_cursor: null });
    return json({ html: "<!doctype html><p>hello page</p>" });
  });
  global.fetch = fetchMock as unknown as typeof fetch;
});

afterEach(() => {
  cleanup();
  global.fetch = originalFetch;
  vi.restoreAllMocks();
});

async function frame(): Promise<HTMLIFrameElement> {
  render(<PageFrame appSlug="scholarships" pageId="board" />);
  return (await screen.findByTestId("page-frame")) as HTMLIFrameElement;
}

/** The posts the frame's window receives, as the page would see them. */
function listen(el: HTMLIFrameElement) {
  const win = el.contentWindow as Window;
  return vi.spyOn(win, "postMessage").mockImplementation(() => {});
}

function fromFrame(el: HTMLIFrameElement, data: unknown) {
  act(() => {
    window.dispatchEvent(new MessageEvent("message", { data, source: el.contentWindow }));
  });
}

const call = (name: string, args: Record<string, unknown>, id = 7) => ({ jsonrpc: "2.0", id, method: "tools/call", params: { name, arguments: args } });
const posts = () => fetchMock.mock.calls.filter(([, init]) => (init as RequestInit | undefined)?.method === "POST");

describe("PageFrame", () => {
  it("loads the page document from the page route and shows it in a frame", async () => {
    const el = await frame();
    expect(fetchMock.mock.calls[0][0]).toBe("/api/v1/apps/scholarships/pages/board");
    expect(el.getAttribute("srcdoc")).toContain("hello page");
  });

  // 1.
  it("sets the sandbox attribute to exactly allow-scripts", async () => {
    const el = await frame();
    expect(el.getAttribute("sandbox")).toBe("allow-scripts");
  });

  // 2.
  it("sends one POST to the page call route for a page_call request from the frame, and answers the frame", async () => {
    const el = await frame();
    const sent = listen(el);
    fromFrame(el, call("page_call", { app_slug: "scholarships", page_id: "board", op: "list", table_id: "applications" }));
    await waitFor(() => expect(posts()).toHaveLength(1));
    const [url, init] = posts()[0] as [string, RequestInit];
    expect(url).toBe("/api/v1/apps/scholarships/pages/board/call");
    expect((init.headers as Record<string, string>).Authorization).toBe("Bearer desk-token");
    expect(JSON.parse(init.body as string)).toMatchObject({ op: "list", table_id: "applications" });
    await waitFor(() => expect(sent).toHaveBeenCalledTimes(1));
    const [reply] = sent.mock.calls[0];
    expect(reply).toMatchObject({ jsonrpc: "2.0", id: 7, result: { structuredContent: { records: [{ id: "r1" }] } } });
  });

  // 3.
  it("refuses any other tool, sends nothing to the server, and says why", async () => {
    const el = await frame();
    const sent = listen(el);
    fromFrame(el, call("update_workspace", { workspace_id: "ws-physics-optics", name: "Hacked" }));
    await waitFor(() => expect(sent).toHaveBeenCalledTimes(1));
    expect(posts()).toHaveLength(0);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(sent.mock.calls[0][0]).toMatchObject({ jsonrpc: "2.0", id: 7, error: { code: expect.any(Number) } });
  });

  it("refuses a request that is not a tools/call, and one with no arguments object", async () => {
    const el = await frame();
    const sent = listen(el);
    fromFrame(el, { jsonrpc: "2.0", id: 1, method: "resources/read", params: { uri: "x" } });
    fromFrame(el, { jsonrpc: "2.0", id: 2, method: "tools/call", params: { name: "page_call" } });
    fromFrame(el, "not json rpc");
    await waitFor(() => expect(sent).toHaveBeenCalledTimes(2));
    expect(posts()).toHaveLength(0);
  });

  // 4.
  it("ignores a message from any window but its own frame", async () => {
    await frame();
    act(() => {
      window.dispatchEvent(new MessageEvent("message", { data: call("page_call", { op: "list", table_id: "applications" }), source: window }));
    });
    await new Promise((r) => setTimeout(r, 50));
    expect(posts()).toHaveLength(0);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  // 5.
  it("forwards a request that names another app or page with this frame's own", async () => {
    const el = await frame();
    listen(el);
    fromFrame(el, call("page_call", { app_slug: "someone-elses-app", page_id: "someone-elses-page", op: "get", table_id: "applications", record_id: "r1" }));
    await waitFor(() => expect(posts()).toHaveLength(1));
    const [url, init] = posts()[0] as [string, RequestInit];
    expect(url).toBe("/api/v1/apps/scholarships/pages/board/call");
    expect(JSON.parse(init.body as string)).toMatchObject({ app_slug: "scholarships", page_id: "board", op: "get" });
  });

  it("gives the page the server's refusal as an error result and never the token", async () => {
    fetchMock.mockImplementation(async (_url: string, init?: RequestInit) => (init?.method === "POST" ? json({ error: "The page has no grant for the table 'notes'" }, 403) : json({ html: "<p>x</p>" })));
    const el = await frame();
    const sent = listen(el);
    fromFrame(el, call("page_call", { op: "list", table_id: "notes" }));
    await waitFor(() => expect(sent).toHaveBeenCalledTimes(1));
    const reply = sent.mock.calls[0][0] as { result: { isError: boolean; content: { text: string }[] } };
    expect(reply.result.isError).toBe(true);
    expect(reply.result.content[0].text).toContain("no grant");
    expect(JSON.stringify(sent.mock.calls)).not.toContain("desk-token");
    expect(el.getAttribute("srcdoc")).not.toContain("desk-token");
  });

  it("shows a message and no frame when the page cannot be loaded", async () => {
    fetchMock.mockImplementation(async () => json({ error: "The page 'board' shows a view" }, 400));
    render(<PageFrame appSlug="scholarships" pageId="board" />);
    expect((await screen.findByRole("alert")).textContent).toMatch(/shows a view/);
    expect(screen.queryByTestId("page-frame")).toBeNull();
  });
});
