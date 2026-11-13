import { useCallback, useEffect, useState } from "react";
import { apiClient, type CreatedShare, type ShareOptions, type ShareRow } from "./api";
import { Banner } from "./ui/Banner";

const buttonClass = "px-3 py-1.5 text-sm font-medium rounded border border-gray-300 dark:border-gray-700 disabled:opacity-50";

/** Who can open a link, in a plain sentence. */
function whoCanOpen(scope: string): string {
  return scope === "public"
    ? "Anyone with the link can open it, with no sign-in. It never shows a sensitive field, and it shows no row that depends on who is looking."
    : "Anyone signed in at the institution who may already read this app can open it. They see only their own rows, and never a sensitive field.";
}

const day = (iso: string) => iso.slice(0, 10);

/**
 * Makes and revokes the shared links of one view. The link is shown once, when it is made: the server keeps only its hash, so
 * a list cannot show it again. Only the scopes a Platform Admin turned on are offered.
 */
export function ShareDialog({ appSlug, viewId, viewName }: { appSlug: string; viewId: string; viewName: string }) {
  const [shares, setShares] = useState<ShareRow[] | null>(null);
  const [options, setOptions] = useState<ShareOptions | null>(null);
  const [scope, setScope] = useState("institution");
  const [days, setDays] = useState("30");
  const [download, setDownload] = useState(false);
  const [made, setMade] = useState<CreatedShare | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      const r = await apiClient.listShares(appSlug, viewId);
      setShares(r.shares);
      setOptions(r.options);
      setDays(String(r.options.default_days));
      setScope((cur) => (r.options.scopes.includes(cur) ? cur : (r.options.scopes[0] ?? "institution")));
    } catch (e) {
      setError(e instanceof Error ? e.message : "The links could not be loaded.");
    }
  }, [appSlug, viewId]);

  useEffect(() => {
    void load();
  }, [load]);

  if (options === null) return error ? <p role="alert" className="text-sm text-red-700">{error}</p> : <p className="text-sm text-slate-500">Loading…</p>;

  const n = Number.parseInt(days, 10);
  const daysOk = Number.isInteger(n) && n >= 1 && n <= options.max_days;
  const offered = ["institution", "public"].filter((s) => options.scopes.includes(s) && (s !== "public" || options.public.allowed));
  const address = made ? `${window.location.origin}${made.link}` : "";

  const create = async () => {
    setBusy(true);
    setError(null);
    try {
      setMade(await apiClient.createShare(appSlug, viewId, { scope, expires_in_days: n, allow_download: download }));
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : "The link could not be made.");
    } finally {
      setBusy(false);
    }
  };

  const revoke = async (s: ShareRow) => {
    setError(null);
    try {
      await apiClient.revokeShare(appSlug, viewId, s.id);
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : "The link could not be revoked.");
    }
  };

  return (
    <section aria-label={`Share ${viewName}`} className="space-y-4 text-sm">
      <h2 className="text-base font-semibold">{`Share ${viewName}`}</h2>
      {error && <p role="alert" className="text-red-700">{error}</p>}

      {offered.length === 0 ? (
        <p>No kind of link is turned on. A Platform Admin turns them on in the settings.</p>
      ) : (
        <fieldset className="space-y-2">
          <legend className="font-medium">Who can open it</legend>
          {offered.map((s) => (
            <label key={s} className="flex items-center gap-2">
              <input type="radio" name="scope" value={s} checked={scope === s} onChange={() => setScope(s)} />
              {s === "public" ? "Public: anyone with the link" : "Institution: people signed in here"}
            </label>
          ))}
          {!options.public.allowed && options.public.reason && <p className="text-slate-600">{options.public.reason}</p>}
          <p data-testid="who-can-open" className="text-slate-700">{whoCanOpen(scope)}</p>
          {scope === "public" && (
            <Banner kind="warning" label="Public link">
              Anyone on the internet who has this link can read this view, until it expires or you revoke it. Pass it on with care.
            </Banner>
          )}
          <label className="flex items-center gap-2">
            Days until it expires
            <input type="number" min={1} max={options.max_days} value={days} onChange={(e) => setDays(e.target.value)} className="w-20 rounded border px-1 py-0.5" />
          </label>
          {!daysOk && <p className="text-red-700">{`A link lasts 1 to ${options.max_days} days.`}</p>}
          <label className="flex items-center gap-2">
            <input type="checkbox" checked={download} onChange={(e) => setDownload(e.target.checked)} />
            Allow download
          </label>
          <button type="button" className={buttonClass} disabled={busy || !daysOk} onClick={create}>
            Create link
          </button>
        </fieldset>
      )}

      {made && (
        <div className="space-y-2 rounded border p-3">
          <p className="font-medium">Your link. It is shown only once.</p>
          <p className="break-all font-mono">{address}</p>
          <button type="button" className={buttonClass} onClick={() => void navigator.clipboard.writeText(address)}>
            Copy link
          </button>
          {made.warnings.map((w) => (
            <Banner key={w} kind="warning">{w}</Banner>
          ))}
        </div>
      )}

      <div>
        <h3 className="mb-1 font-medium">Active links</h3>
        {(shares ?? []).filter((s) => s.state === "Active").length === 0 ? (
          <p className="text-slate-600">No active links.</p>
        ) : (
          <ul className="space-y-1">
            {(shares ?? []).filter((s) => s.state === "Active").map((s) => (
              <li key={s.id} className="flex flex-wrap items-center gap-3">
                <span>{s.scope === "public" ? "Public" : "Institution"}</span>
                <span className="text-slate-600">{`expires ${day(s.expires_at)}, opened ${s.use_count} times`}</span>
                <button type="button" className={buttonClass} aria-label={`Revoke link created ${day(s.created_at)}`} onClick={() => revoke(s)}>
                  Revoke
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}
