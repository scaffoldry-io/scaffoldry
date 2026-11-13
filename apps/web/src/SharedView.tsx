import { useEffect, useState } from "react";
import { apiClient, type SharedViewInfo, type ViewData } from "./api";

const cell = (v: unknown): string => (v === null || v === undefined ? "" : typeof v === "object" ? JSON.stringify(v) : String(v));

/**
 * What a shared link opens: one view, read only. It holds no sensitive or hidden field, because the server never sends
 * one. The link is the only credential, and a link for the institution also needs the person to be signed in.
 */
export function SharedView({ token }: { token: string }) {
  const [info, setInfo] = useState<SharedViewInfo | null>(null);
  const [data, setData] = useState<ViewData | null>(null);
  const [problem, setProblem] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    Promise.all([apiClient.sharedView(token), apiClient.sharedData(token, { limit: 100 })])
      .then(([i, d]) => {
        if (live) {
          setInfo(i);
          setData(d);
        }
      })
      .catch((e: unknown) => live && setProblem(e instanceof Error ? e.message : "This link could not be opened."));
    return () => {
      live = false;
    };
  }, [token]);

  const download = async () => {
    try {
      const csv = await apiClient.sharedDownload(token);
      const url = URL.createObjectURL(new Blob([csv], { type: "text/csv" }));
      const a = document.createElement("a");
      a.href = url;
      a.download = `${info?.name ?? "view"}.csv`;
      a.click();
      URL.revokeObjectURL(url);
    } catch (e) {
      setProblem(e instanceof Error ? e.message : "The download failed.");
    }
  };

  if (problem) return <p role="alert" className="p-6 text-sm text-red-700">{problem}</p>;
  if (!info || !data) return <p className="p-6 text-sm text-slate-500">Loading…</p>;

  const records = data.records ?? [];
  const columns = records.length ? Object.keys(records[0].data).filter((k) => !k.startsWith("_")) : [];
  return (
    <main className="mx-auto max-w-5xl space-y-3 p-6">
      <header>
        <h1 className="text-lg font-semibold">{info.name}</h1>
        <p className="text-xs text-slate-600">{`${info.app_title ?? "Shared view"}. Read only. This link closes on ${info.expires_at.slice(0, 10)}.`}</p>
      </header>
      {info.allow_download && (
        <button type="button" onClick={download} className="rounded border px-3 py-1.5 text-sm font-medium">
          Download
        </button>
      )}
      {records.length === 0 ? (
        <p className="text-sm text-slate-600">There are no rows to show.</p>
      ) : (
        <table className="w-full text-sm">
          <thead>
            <tr>
              {columns.map((c) => (
                <th key={c} scope="col" className="border-b px-2 py-1 text-left">{c}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {records.map((r) => (
              <tr key={r.id}>
                {columns.map((c) => (
                  <td key={c} className="px-2 py-1">{cell(r.data[c])}</td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </main>
  );
}
