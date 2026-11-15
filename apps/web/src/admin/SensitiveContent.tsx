import { useCallback, useEffect, useMemo, useState } from "react";
import { apiClient, type AdminFieldRow, type AuthorizedStore, type ContentFinding, type DetectorStat, type SensitivityDetector, type SensitivitySettings } from "../api";
import { ConfirmAction, Drawer, EmptyState, ErrorState, PageHeader, Skeleton, Tabs } from "../ui";

const buttonClass = "px-3 py-1.5 text-sm font-medium rounded border border-gray-300 dark:border-gray-700 disabled:opacity-50";
const when = (iso: string | null) => (iso ? (Number.isNaN(new Date(iso).getTime()) ? iso : new Date(iso).toLocaleString("en-US", { dateStyle: "medium", timeStyle: "short" })) : "never");

// ------------------------------------------------------------------------------------------------------------------------------
// Findings
// ------------------------------------------------------------------------------------------------------------------------------

function Findings() {
  const [rows, setRows] = useState<ContentFinding[] | null>(null);
  const [next, setNext] = useState<string | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [status, setStatus] = useState("open");
  const [category, setCategory] = useState("");
  const [review, setReview] = useState<ContentFinding | null>(null);
  const [deciding, setDeciding] = useState<"confirm" | "dismiss" | null>(null);
  const [scanned, setScanned] = useState(false);

  const load = useCallback(
    async (cursor?: string) => {
      setError(null);
      try {
        const page = await apiClient.contentFindings({ status, category, ...(cursor ? { cursor } : {}) });
        setRows((old) => (cursor && old ? [...old, ...page.findings] : page.findings));
        setNext(page.next_cursor);
      } catch (e) {
        setError(e);
        setRows([]);
      }
    },
    [status, category]
  );
  useEffect(() => {
    void load();
  }, [load]);

  if (error !== null) return <ErrorState error={error} />;
  if (rows === null) return <Skeleton rows={4} />;
  const close = () => {
    setReview(null);
    setDeciding(null);
  };

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-end gap-3">
        <div>
          <label htmlFor="sc-status" className="block text-xs text-gray-500">Show</label>
          <select id="sc-status" value={status} onChange={(e) => setStatus(e.target.value)} className="rounded border px-2 py-1.5 text-sm">
            <option value="open">Open findings</option>
            <option value="confirmed">Confirmed</option>
            <option value="dismissed">Dismissed</option>
            <option value="">All</option>
          </select>
        </div>
        <div>
          <label htmlFor="sc-category" className="block text-xs text-gray-500">Category</label>
          <input id="sc-category" value={category} onChange={(e) => setCategory(e.target.value)} className="rounded border px-2 py-1.5 text-sm" placeholder="pii" />
        </div>
        <button
          type="button"
          className={buttonClass}
          onClick={() => apiClient.scanContent({ all: true }).then(() => setScanned(true), (e: unknown) => setError(e))}
        >
          Scan every table
        </button>
        {scanned && <span role="status" className="text-sm text-gray-600">Scans are queued. Findings appear as they finish.</span>}
      </div>

      {rows.length === 0 ? (
        <EmptyState title="No findings" description="A scan found nothing here that needs a decision." />
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full text-sm" aria-label="Findings">
            <thead>
              <tr className="text-left text-xs text-gray-500">
                <th className="pr-3 py-1">App</th>
                <th className="pr-3">Table</th>
                <th className="pr-3">Field</th>
                <th className="pr-3">Detector</th>
                <th className="pr-3">Category</th>
                <th className="pr-3">Matches</th>
                <th className="pr-3">Records</th>
                <th className="pr-3">State</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {rows.map((f) => (
                <tr key={f.id} className="border-t border-gray-200 dark:border-gray-800">
                  <td className="pr-3 py-1">{f.app_slug}</td>
                  <td className="pr-3">{f.table_id}</td>
                  <td className="pr-3">{f.field}</td>
                  <td className="pr-3 text-xs">{f.detector_id}</td>
                  <td className="pr-3">{f.category}</td>
                  <td className="pr-3">{f.match_count}</td>
                  <td className="pr-3">{f.record_count}</td>
                  <td className="pr-3">{f.status}</td>
                  <td>
                    <button type="button" className={buttonClass} aria-label={`Review ${f.app_slug} ${f.table_id} ${f.field}`} onClick={() => setReview(f)}>
                      Review
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {next && (
        <button type="button" className={buttonClass} onClick={() => void load(next)}>
          Load more
        </button>
      )}

      <Drawer open={review !== null} onClose={close} title={review ? `${review.field} in ${review.table_id}` : "Finding"}>
        {review && (
          <div className="space-y-3 text-sm">
            <p>
              {`${review.match_count} matches of ${review.detector_id} in ${review.record_count} records. Category: ${review.category}. First seen ${when(review.first_seen)}.`}
            </p>
            <div>
              <h4 className="text-xs font-medium text-gray-500">Records to look at</h4>
              {review.sample_record_ids.length === 0 ? (
                <p>None kept.</p>
              ) : (
                <ul>
                  {review.sample_record_ids.map((id) => (
                    <li key={id}>
                      <a className="underline" href={`/app/${encodeURIComponent(review.app_slug)}?record=${encodeURIComponent(id)}`}>
                        {id}
                      </a>
                    </li>
                  ))}
                </ul>
              )}
              <p className="text-xs text-gray-600">Each link opens the record under the usual access rules. The finding itself holds no value.</p>
            </div>
            {review.status !== "open" ? (
              <p>{`This finding was ${review.status}${review.reason ? `: ${review.reason}` : ""}.`}</p>
            ) : deciding === null ? (
              <div className="flex gap-2">
                <button type="button" className={buttonClass} onClick={() => setDeciding("confirm")}>
                  Confirm…
                </button>
                <button type="button" className={buttonClass} onClick={() => setDeciding("dismiss")}>
                  Dismiss…
                </button>
              </div>
            ) : (
              <ConfirmAction
                verb={deciding === "confirm" ? "Confirm" : "Dismiss"}
                target={review.field}
                consequence={
                  deciding === "confirm"
                    ? "This field will be treated as sensitive. Exports and reads will be restricted."
                    : "The field stays as it is. This finding will not come back until its detector changes."
                }
                onConfirm={(reason) => apiClient.decideFinding(review.id, deciding, reason)}
                onSuccess={() => {
                  close();
                  void load();
                }}
              />
            )}
          </div>
        )}
      </Drawer>
    </div>
  );
}

// ------------------------------------------------------------------------------------------------------------------------------
// Categories
// ------------------------------------------------------------------------------------------------------------------------------

function Categories() {
  const [data, setData] = useState<SensitivitySettings | null>(null);
  const [stats, setStats] = useState<DetectorStat[]>([]);
  const [error, setError] = useState<unknown>(null);
  const [change, setChange] = useState<{ detector: SensitivityDetector; action: SensitivityDetector["action"] } | null>(null);
  const [problem, setProblem] = useState<string | null>(null);

  const load = useCallback(async () => {
    setError(null);
    try {
      const [s, st] = await Promise.all([apiClient.sensitivitySettings(), apiClient.detectorStats()]);
      setData(s);
      setStats(st);
    } catch (e) {
      setError(e);
    }
  }, []);
  useEffect(() => {
    void load();
  }, [load]);

  if (error !== null) return <ErrorState error={error} />;
  if (data === null) return <Skeleton rows={4} />;

  const save = async (d: SensitivityDetector, action: SensitivityDetector["action"], reason?: string) => {
    setProblem(null);
    const list = data.detectors.map((x) => (x.id === d.id ? { ...x, action } : x));
    await apiClient.updateSetting("sensitivity.detectors", list, reason);
    setChange(null);
    await load();
  };
  const statOf = (id: string) => stats.find((s) => s.detector_id === id);
  const share = (n: number) => `${Math.round(n * 100)}%`;

  return (
    <div className="space-y-6">
      {problem && <p role="alert" className="text-sm text-red-700">{problem}</p>}
      <section aria-labelledby="sc-cats" className="space-y-2">
        <h3 id="sc-cats" className="text-sm font-semibold">Categories</h3>
        <ul className="text-sm">
          {data.categories.map((c) => (
            <li key={c.id}>{`${c.name} (${c.id}). ${c.protected ? "Protected" : "Not protected"}. Source: ${c.source || "none"}. Detectors: ${c.detectors.join(", ") || "none"}.`}</li>
          ))}
        </ul>
        <h4 className="text-xs font-medium text-gray-500">Presets</h4>
        <ul className="space-y-1 text-sm">
          {data.presets.map((p) => (
            <li key={p.id} className="flex items-center gap-3">
              <span>{p.name}</span>
              {p.switched_on ? (
                <span className="text-xs text-gray-600">On</span>
              ) : (
                <button
                  type="button"
                  className={buttonClass}
                  aria-label={`Switch on ${p.name}`}
                  onClick={() => apiClient.enableSensitivityPreset(p.id).then(load, (e: unknown) => setProblem(e instanceof Error ? e.message : "That could not be switched on."))}
                >
                  Switch on
                </button>
              )}
            </li>
          ))}
        </ul>
      </section>

      <section aria-labelledby="sc-det" className="space-y-2">
        <h3 id="sc-det" className="text-sm font-semibold">Detectors</h3>
        <div className="overflow-x-auto">
          <table className="w-full text-sm" aria-label="Detectors">
            <thead>
              <tr className="text-left text-xs text-gray-500">
                <th className="pr-3 py-1">Detector</th>
                <th className="pr-3">On</th>
                <th className="pr-3">Action</th>
                <th className="pr-3">Findings</th>
                <th className="pr-3">Confirmed</th>
                <th className="pr-3">Dismissed</th>
              </tr>
            </thead>
            <tbody>
              {data.detectors.map((d) => {
                const st = statOf(d.id);
                return (
                  <tr key={d.id} className="border-t border-gray-200 dark:border-gray-800">
                    <td className="pr-3 py-1">{d.id}</td>
                    <td className="pr-3">{d.enabled ? "Yes" : "No"}</td>
                    <td className="pr-3">
                      <select
                        aria-label={`Action of ${d.id}`}
                        value={d.action}
                        className="rounded border px-2 py-1 text-sm"
                        onChange={(e) => {
                          const action = e.target.value as SensitivityDetector["action"];
                          if (action === "block") setChange({ detector: d, action });
                          else void save(d, action).catch((err: unknown) => setProblem(err instanceof Error ? err.message : "That could not be saved."));
                        }}
                      >
                        <option value="flag">flag</option>
                        <option value="warn">warn</option>
                        <option value="block">block</option>
                      </select>
                    </td>
                    <td className="pr-3">{st?.findings ?? 0}</td>
                    <td className="pr-3">{st?.confirmed ?? 0}</td>
                    <td className="pr-3">{st ? `${st.dismissed} (${share(st.dismissed_share)} of decided)` : "0"}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
        {change && (
          <div className="rounded border p-3">
            <p className="mb-2 text-sm">
              {`${statOf(change.detector.id)?.dismissed ?? 0} findings of ${change.detector.id} were dismissed, ${share(statOf(change.detector.id)?.dismissed_share ?? 0)} of those decided. A detector that is mostly dismissed is not ready to block.`}
            </p>
            <ConfirmAction
              verb="Block with"
              target={change.detector.id}
              consequence="A write that matches this detector outside an authorized store will be refused."
              onConfirm={(reason) => save(change.detector, change.action, reason)}
              onSuccess={() => setChange(null)}
            />
          </div>
        )}
      </section>
    </div>
  );
}

// ------------------------------------------------------------------------------------------------------------------------------
// Authorized stores
// ------------------------------------------------------------------------------------------------------------------------------

type Place = { label: string; body: { scope: AuthorizedStore["scope"]; workspace_id?: string; app_slug?: string; table_id?: string; field?: string } };

function Stores() {
  const [stores, setStores] = useState<AuthorizedStore[] | null>(null);
  const [categories, setCategories] = useState<string[]>([]);
  const [fields, setFields] = useState<AdminFieldRow[]>([]);
  const [error, setError] = useState<unknown>(null);
  const [removing, setRemoving] = useState<AuthorizedStore | null>(null);
  const [adding, setAdding] = useState(false);
  const [category, setCategory] = useState("");
  const [scope, setScope] = useState<AuthorizedStore["scope"]>("field");
  const [find, setFind] = useState("");
  const [place, setPlace] = useState<Place | null>(null);

  const load = useCallback(async () => {
    setError(null);
    try {
      const [s, cfg, f] = await Promise.all([apiClient.authorizedStores(), apiClient.sensitivitySettings(), apiClient.adminFields({ limit: "500" })]);
      setStores(s);
      setCategories(cfg.categories.map((c) => c.id));
      setFields(f.fields);
    } catch (e) {
      setError(e);
    }
  }, []);
  useEffect(() => {
    void load();
  }, [load]);

  const places: Place[] = useMemo(() => {
    const out = new Map<string, Place>();
    for (const f of fields) {
      if (scope === "app") out.set(f.app, { label: f.app_title, body: { scope, app_slug: f.app } });
      if (scope === "table") out.set(`${f.app}/${f.table}`, { label: `${f.app_title}, ${f.table}`, body: { scope, app_slug: f.app, table_id: f.table } });
      if (scope === "field") out.set(`${f.app}/${f.table}/${f.field}`, { label: `${f.app_title}, ${f.table}, ${f.field}`, body: { scope, app_slug: f.app, table_id: f.table, field: f.field } });
    }
    return [...out.values()].filter((p) => p.label.toLowerCase().includes(find.trim().toLowerCase())).slice(0, 50);
  }, [fields, scope, find]);

  if (error !== null) return <ErrorState error={error} />;
  if (stores === null) return <Skeleton rows={3} />;
  const words = (s: AuthorizedStore) => {
    const where = s.scope === "workspace" ? `workspace ${s.workspace_id}` : s.scope === "app" ? `app ${s.app_slug}` : s.scope === "table" ? `table ${s.table_id} of ${s.app_slug}` : `field ${s.field} of ${s.table_id} in ${s.app_slug}`;
    return `${s.category} may live in ${where}`;
  };

  return (
    <div className="space-y-3">
      {stores.length === 0 ? (
        <EmptyState title="No authorized stores" description="Until a place is designated, a match of any detector anywhere raises a finding." />
      ) : (
        <ul className="space-y-1 text-sm" aria-label="Authorized stores">
          {stores.map((s) => (
            <li key={`${s.category}/${s.scope}/${s.workspace_id}/${s.app_slug}/${s.table_id}/${s.field}`} className="flex items-center gap-3">
              <span>{words(s)}</span>
              <button type="button" className={buttonClass} aria-label={`Remove ${words(s)}`} onClick={() => setRemoving(s)}>
                Remove
              </button>
            </li>
          ))}
        </ul>
      )}
      {removing && (
        <div className="rounded border p-3">
          <ConfirmAction
            verb="Remove"
            target={words(removing)}
            consequence="Matches in this place raise findings again. The labels stay until a person removes them."
            onConfirm={(reason) => apiClient.removeAuthorizedStore({ category: removing.category, scope: removing.scope, workspace_id: removing.workspace_id, app_slug: removing.app_slug, table_id: removing.table_id, field: removing.field }, reason)}
            onSuccess={() => {
              setRemoving(null);
              void load();
            }}
          />
        </div>
      )}
      {!adding ? (
        <button type="button" className={buttonClass} onClick={() => setAdding(true)}>
          Add
        </button>
      ) : (
        <div className="space-y-2 rounded border p-3" role="group" aria-label="Add an authorized store">
          <div className="flex flex-wrap gap-3">
            <div>
              <label htmlFor="as-cat" className="block text-xs text-gray-500">Category</label>
              <select id="as-cat" value={category} onChange={(e) => setCategory(e.target.value)} className="rounded border px-2 py-1.5 text-sm">
                <option value="">Choose…</option>
                {categories.map((c) => (
                  <option key={c} value={c}>{c}</option>
                ))}
              </select>
            </div>
            <div>
              <label htmlFor="as-scope" className="block text-xs text-gray-500">Kind of place</label>
              <select id="as-scope" value={scope} onChange={(e) => { setScope(e.target.value as AuthorizedStore["scope"]); setPlace(null); }} className="rounded border px-2 py-1.5 text-sm">
                <option value="app">An app</option>
                <option value="table">A table</option>
                <option value="field">A field</option>
              </select>
            </div>
            <div>
              <label htmlFor="as-find" className="block text-xs text-gray-500">Find the place</label>
              <input id="as-find" value={find} onChange={(e) => setFind(e.target.value)} className="rounded border px-2 py-1.5 text-sm" />
            </div>
          </div>
          <label htmlFor="as-place" className="block text-xs text-gray-500">Place</label>
          <select id="as-place" size={6} value={place?.label ?? ""} onChange={(e) => setPlace(places.find((p) => p.label === e.target.value) ?? null)} className="w-full rounded border px-2 py-1 text-sm">
            {places.map((p) => (
              <option key={p.label} value={p.label}>{p.label}</option>
            ))}
          </select>
          {category && place ? (
            <ConfirmAction
              verb="Designate"
              target={place.label}
              consequence={`The fields there are labeled ${category} at once, and a match of ${category} there raises no finding.`}
              onConfirm={(reason) => apiClient.putAuthorizedStore({ category, ...place.body }, reason)}
              onSuccess={() => {
                setAdding(false);
                setPlace(null);
                void load();
              }}
            />
          ) : (
            <p className="text-xs text-gray-600">Choose a category and a place.</p>
          )}
        </div>
      )}
    </div>
  );
}

// The Sensitive content section of the console (sensitive content phase 2). A scan writes findings and never a label. A person decides.
export function SensitiveContent() {
  const [tab, setTab] = useState<"findings" | "categories" | "stores">("findings");
  return (
    <div className="space-y-4">
      <PageHeader title="Sensitive content" description="Scans find patterns such as Social Security numbers and card numbers in stored records. A finding holds counts and record ids, never a value. A person confirms or dismisses it." />
      <Tabs
        ariaLabel="Sensitive content"
        selectedId={tab}
        onSelect={(id) => setTab(id as typeof tab)}
        tabs={[
          { id: "findings", label: "Findings" },
          { id: "categories", label: "Categories" },
          { id: "stores", label: "Authorized stores" },
        ]}
      />
      {tab === "findings" ? <Findings /> : tab === "categories" ? <Categories /> : <Stores />}
    </div>
  );
}
