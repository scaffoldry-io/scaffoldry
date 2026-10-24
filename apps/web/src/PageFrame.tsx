import { useCallback, useEffect, useRef, useState } from "react";
import { ApiError, callPage, getPage } from "./api";

/**
 * Shows a custom page in a sandboxed frame.
 *
 * The frame is `allow-scripts` and nothing else, so it has no origin of its own, no storage, no way to reach this
 * window's cookies or the token, and cannot navigate or open anything. It talks to the server only by asking this
 * component, with one request: a JSON-RPC `tools/call` of `page_call`. The component forwards that call to this page's
 * own route, with this page's own app and page ids, and sends the answer back. The server decides what the page may do.
 *
 * A page cannot approve a proposal or decide a process step. Those buttons are drawn by the desk, outside the frame.
 */
export function PageFrame({ appSlug, pageId }: { appSlug: string; pageId: string }) {
  const frame = useRef<HTMLIFrameElement>(null);
  const [html, setHtml] = useState<string | null>(null);
  const [problem, setProblem] = useState<string | null>(null);

  useEffect(() => {
    let current = true;
    setHtml(null);
    setProblem(null);
    getPage(appSlug, pageId)
      .then((page) => current && setHtml(page.html))
      .catch((e: unknown) => current && setProblem(e instanceof Error ? e.message : "The page could not be loaded."));
    return () => {
      current = false;
    };
  }, [appSlug, pageId]);

  const reply = useCallback((message: Record<string, unknown>) => {
    // The frame has an opaque origin, so "*" is the only target that reaches it. Only this frame's window is addressed.
    frame.current?.contentWindow?.postMessage({ jsonrpc: "2.0", ...message }, "*");
  }, []);

  useEffect(() => {
    const onMessage = async (event: MessageEvent) => {
      // Anything that did not come from this frame is not a page's request.
      if (!frame.current || event.source !== frame.current.contentWindow) return;
      const request = event.data as { jsonrpc?: string; id?: unknown; method?: string; params?: { name?: string; arguments?: unknown } } | null;
      if (!request || typeof request !== "object" || request.jsonrpc !== "2.0" || request.id === undefined) return;
      // MCP Apps (specification 2026-01-26): the page says ui/initialize and waits for this answer before it sends a tool
      // call. The desk offers no host capability. `ui/notifications/initialized` has no id, so it is ignored above.
      if (request.method === "ui/initialize") {
        reply({ id: request.id, result: { protocolVersion: "2026-01-26", hostCapabilities: {}, hostInfo: { name: "scaffoldry-desk", version: "1" }, hostContext: {} } });
        return;
      }
      if (request.method !== "tools/call") {
        reply({ id: request.id, error: { code: -32601, message: "A page may only call tools." } });
        return;
      }
      const args = request.params?.arguments;
      if (request.params?.name !== "page_call" || !args || typeof args !== "object" || Array.isArray(args)) {
        reply({ id: request.id, error: { code: -32602, message: "A page may only call page_call, with an arguments object." } });
        return;
      }
      // This frame's own app and page, whatever the page named.
      const own = { ...(args as Record<string, unknown>), app_slug: appSlug, page_id: pageId };
      try {
        const result = await callPage(appSlug, pageId, own);
        reply({ id: request.id, result: { structuredContent: result } });
      } catch (e) {
        const message = e instanceof ApiError || e instanceof Error ? e.message : "The call failed.";
        reply({ id: request.id, result: { isError: true, content: [{ type: "text", text: message }] } });
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [appSlug, pageId, reply]);

  if (problem) {
    return (
      <div role="alert" className="rounded-md border border-red-200 bg-red-50 p-4 text-sm text-red-800">
        {problem}
      </div>
    );
  }
  if (html === null) return <div className="p-4 text-sm text-slate-500">Loading the page…</div>;
  return (
    <iframe
      ref={frame}
      title="Custom page"
      data-testid="page-frame"
      // Exactly this, and never allow-same-origin, allow-top-navigation, allow-popups, or allow-forms.
      sandbox="allow-scripts"
      srcDoc={html}
      className="h-full min-h-[480px] w-full border-0"
    />
  );
}
