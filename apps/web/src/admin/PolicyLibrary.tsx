import { useCallback, useEffect, useState, type FormEvent, type ReactNode } from "react";
import { apiClient, type AdminLibraryPolicy, type LibraryHealth, type SourceComparison } from "../api";
import { Banner, EmptyState, ErrorState, PageHeader, Skeleton } from "../ui";

const buttonClass = "px-3 py-1.5 text-sm font-medium rounded border border-gray-300 dark:border-gray-700 disabled:opacity-50";
const inputClass = "w-full text-sm rounded border border-gray-300 dark:border-gray-700 p-2 bg-white dark:bg-gray-800";

/** A policy's owner in words a person can act on. */
function ownerText(owner: Record<string, any>): string {
  if (owner?.Person?.eppn) return owner.Person.eppn;
  if (owner?.Position?.key) return `the ${owner.Position.key} position`;
  if (owner?.Role?.name) return `the ${owner.Role.name} role`;
  return "Not set";
}

function day(iso: string | null | undefined): string {
  if (!iso) return "never";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleDateString("en-US", { dateStyle: "medium" });
}

/** "3 published, 1 draft", in a fixed order. */
function ruleCounts(counts: Record<string, number> | undefined): string {
  const order = ["Published", "InReview", "Draft", "Rejected", "Superseded", "Retired"];
  const words: Record<string, string> = { Published: "published", InReview: "in review", Draft: "draft", Rejected: "rejected", Superseded: "superseded", Retired: "retired" };
  const parts = order.filter((s) => (counts?.[s] ?? 0) > 0).map((s) => `${counts![s]} ${words[s]}`);
  return parts.length ? parts.join(", ") : "No rules";
}

/** A link inside the console: it moves the console, not the page. */
function Go({ to, navigate, children }: { to: string; navigate: (to: string) => void; children: ReactNode }) {
  return (
    <a
      href={to}
      className="text-blue-700 dark:text-blue-300 underline"
      onClick={(e) => {
        e.preventDefault();
        navigate(to);
      }}
    >
      {children}
    </a>
  );
}

// ---------------------------------------------------------------------------------------------------------------------
// The list of policies
// ---------------------------------------------------------------------------------------------------------------------

function NewPolicy({ onDone, onCancel }: { onDone: () => void; onCancel: () => void }) {
  const [d, setD] = useState({ key: "", title: "", unit: "", owner: "", reason: "" });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const set = (p: Partial<typeof d>) => setD((x) => ({ ...x, ...p }));
  const ready = d.key.trim() && d.title.trim() && d.unit.trim() && d.owner.trim() && d.reason.trim();
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await apiClient.createLibraryPolicy({ key: d.key.trim(), title: d.title.trim(), organization_id: d.unit.trim(), owner: { Person: { eppn: d.owner.trim() } }, reason: d.reason.trim() });
      onDone();
    } catch (err) {
      setError(err);
    } finally {
      setBusy(false);
    }
  };
  const field = (id: string, label: string, value: string, key: keyof typeof d, hint?: string) => (
    <label htmlFor={id} className="block text-xs font-medium">
      {label}
      <input id={id} className={inputClass} value={value} onChange={(e) => set({ [key]: e.target.value } as Partial<typeof d>)} placeholder={hint} />
    </label>
  );
  return (
    <form onSubmit={(e) => void submit(e)} className="space-y-3 rounded border border-gray-200 dark:border-gray-800 p-3">
      {error !== null && <ErrorState error={error} />}
      {field("policy-key", "Key", d.key, "key", "records-retention")}
      {field("policy-title", "Title", d.title, "title")}
      {field("policy-unit", "Unit", d.unit, "unit", "The unit's id")}
      {field("policy-owner", "Owner", d.owner, "owner", "The owner's email address")}
      <label htmlFor="policy-reason" className="block text-xs font-medium">
        Reason
        <textarea id="policy-reason" rows={2} className={inputClass} value={d.reason} onChange={(e) => set({ reason: e.target.value })} />
      </label>
      <div className="flex gap-2">
        <button type="submit" className={buttonClass} disabled={busy || !ready}>
          Create policy
        </button>
        <button type="button" className={buttonClass} onClick={onCancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}

function PolicyList({ navigate }: { navigate: (to: string) => void }) {
  const [rows, setRows] = useState<AdminLibraryPolicy[] | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [adding, setAdding] = useState(false);
  const load = useCallback(async () => {
    setError(null);
    try {
      setRows(await apiClient.listAdminLibraryPolicies());
    } catch (e) {
      setError(e);
      setRows([]);
    }
  }, []);
  useEffect(() => {
    void load();
  }, [load]);
  return (
    <div className="space-y-3">
      {error !== null && <ErrorState error={error} />}
      {!adding && (
        <button type="button" className={buttonClass} onClick={() => setAdding(true)}>
          New policy
        </button>
      )}
      {adding && (
        <NewPolicy
          onDone={() => {
            setAdding(false);
            void load();
          }}
          onCancel={() => setAdding(false)}
        />
      )}
      {rows === null ? (
        <Skeleton rows={4} />
      ) : rows.length === 0 ? (
        <EmptyState title="No policies" description="Add a policy, then its sources, then the rules its sources say." />
      ) : (
        <table className="w-full text-sm">
          <thead>
            <tr className="text-left text-xs text-gray-500">
              <th className="py-1">Policy</th>
              <th>Owner</th>
              <th>Unit</th>
              <th>Rules</th>
              <th>Open questions</th>
              <th>Review</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((p) => (
              <tr key={p.id} className="border-t border-gray-100 dark:border-gray-800">
                <td className="py-1.5">
                  <Go to={`/admin/library/${p.id}`} navigate={navigate}>
                    {p.title}
                  </Go>
                </td>
                <td>{ownerText(p.owner)}</td>
                <td>{p.unit_name ?? p.organization_id}</td>
                <td>{ruleCounts(p.rule_counts)}</td>
                <td>{p.open_questions ?? 0}</td>
                <td>{p.due ? <span className="text-amber-700 dark:text-amber-300 font-medium">Review due</span> : `Reviewed ${day(p.last_reviewed_at)}`}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------------------------------------------------
// The state of the library
// ---------------------------------------------------------------------------------------------------------------------

function Section({ title, count, empty, children }: { title: string; count: number; empty: string; children: ReactNode }) {
  return (
    <section className="space-y-1">
      <h2 className="text-sm font-semibold">{`${title} (${count.toLocaleString("en-US")})`}</h2>
      {count === 0 ? <p className="text-xs text-gray-500">{empty}</p> : <ul className="space-y-1 text-sm">{children}</ul>}
    </section>
  );
}

const PROBLEM: Record<string, string> = {
  unreachable: "unreachable: three fetches failed in a row",
  needs_text: "needs text: the page is not text the platform can read",
  changed: "changed: a cited passage is gone",
};

function Health({ navigate }: { navigate: (to: string) => void }) {
  const [h, setH] = useState<LibraryHealth | null>(null);
  const [error, setError] = useState<unknown>(null);
  useEffect(() => {
    let live = true;
    apiClient.libraryHealth().then((x) => live && setH(x)).catch((e) => live && setError(e));
    return () => {
      live = false;
    };
  }, []);
  if (error !== null) return <ErrorState error={error} />;
  if (!h) return <Skeleton rows={6} />;
  const more = (shown: number, count: number) => (count > shown ? <li className="text-xs text-gray-500">{`and ${(count - shown).toLocaleString("en-US")} more`}</li> : null);
  return (
    <div className="space-y-5">
      <Section title="No published rule" count={h.no_published_rule.count} empty="Every policy has a published rule.">
        {h.no_published_rule.items.map((i) => (
          <li key={i.policy_id}>
            <Go to={i.link} navigate={navigate}>
              {i.policy_key}
            </Go>{" "}
            {i.title}
          </li>
        ))}
        {more(h.no_published_rule.items.length, h.no_published_rule.count)}
      </Section>
      <Section title="Guidance only" count={h.guidance_only.count} empty="Every published rule is enforced by something that exists.">
        {h.guidance_only.items.map((i) => (
          <li key={i.rule_id}>
            <Go to={i.link} navigate={navigate}>
              {`${i.policy_key} / ${i.rule_key}`}
            </Go>{" "}
            <span className="text-xs text-gray-500">{i.why}</span>
          </li>
        ))}
        {more(h.guidance_only.items.length, h.guidance_only.count)}
      </Section>
      <Section title="Sources to look at" count={h.sources.count} empty="Every source was read and still holds what its rules cite.">
        {h.sources.items.map((i) => (
          <li key={i.source_id}>
            <Go to={i.link} navigate={navigate}>
              {`${i.policy_key}: ${i.title}`}
            </Go>{" "}
            <span className="text-xs text-gray-500">{PROBLEM[i.problem ?? ""] ?? i.problem}</span>
          </li>
        ))}
        {more(h.sources.items.length, h.sources.count)}
      </Section>
      <Section title="Reviews due" count={h.review.count} empty="No policy is past its review date, and no question has waited over 30 days.">
        {h.review.items.map((i) => (
          <li key={i.policy_id}>
            <Go to={i.link} navigate={navigate}>
              {i.policy_key}
            </Go>{" "}
            <span className="text-xs text-gray-500">
              {[i.past_review ? "past its review date" : null, i.oldest_open_question_days != null && i.oldest_open_question_days > 30 ? `a question open ${i.oldest_open_question_days} days` : null]
                .filter(Boolean)
                .join(", ")}
            </span>
          </li>
        ))}
        {more(h.review.items.length, h.review.count)}
      </Section>
    </div>
  );
}

// ---------------------------------------------------------------------------------------------------------------------
// A changed source, side by side
// ---------------------------------------------------------------------------------------------------------------------

/** Splits text into plain and marked parts, marking every place a cited passage occurs. */
function marked(text: string, passages: string[]): ReactNode[] {
  const hits: [number, number][] = [];
  for (const p of passages) {
    if (!p) continue;
    let at = text.indexOf(p);
    while (at >= 0) {
      hits.push([at, at + p.length]);
      at = text.indexOf(p, at + p.length);
    }
  }
  hits.sort((a, b) => a[0] - b[0]);
  const out: ReactNode[] = [];
  let pos = 0;
  hits.forEach(([start, end], n) => {
    if (start < pos) return;
    if (start > pos) out.push(text.slice(pos, start));
    out.push(
      <mark key={n} className="bg-amber-200 dark:bg-amber-700/60">
        {text.slice(start, end)}
      </mark>
    );
    pos = end;
  });
  if (pos < text.length) out.push(text.slice(pos));
  return out;
}

export function SourceCompare({ sourceId }: { sourceId: string }) {
  const [c, setC] = useState<SourceComparison | null>(null);
  const [error, setError] = useState<unknown>(null);
  useEffect(() => {
    let live = true;
    apiClient.compareLibrarySource(sourceId).then((x) => live && setC(x)).catch((e) => live && setError(e));
    return () => {
      live = false;
    };
  }, [sourceId]);
  if (error !== null) return <ErrorState error={error} />;
  if (!c) return <Skeleton rows={3} />;
  if (!c.old || !c.new) return <p className="text-sm text-gray-500">There is only one copy of this source so far. There is nothing earlier to compare it with.</p>;
  const supplied = c.old.text_origin === "supplied" || c.new.text_origin === "supplied";
  const pane = "rounded border border-gray-200 dark:border-gray-800 p-3 text-sm whitespace-pre-wrap max-h-96 overflow-auto";
  return (
    <div className="space-y-2">
      {supplied && (
        <Banner kind="warning">This text was supplied by a person, not fetched. Check it against the original file before you rely on it.</Banner>
      )}
      <p className="text-xs text-gray-500">The passages that published rules cite are marked in the earlier copy.</p>
      <div className="grid gap-3 md:grid-cols-2">
        <section role="region" aria-label="Before" className="space-y-1">
          <h3 className="text-xs font-semibold">{`Before, taken ${day(c.old.taken_at)}`}</h3>
          <div className={pane}>{marked(c.old.untrusted_text, c.cited)}</div>
        </section>
        <section role="region" aria-label="After" className="space-y-1">
          <h3 className="text-xs font-semibold">{`After, taken ${day(c.new.taken_at)}`}</h3>
          <div className={pane}>{c.new.untrusted_text}</div>
        </section>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------------------------------------------------
// One policy
// ---------------------------------------------------------------------------------------------------------------------

function EditPolicy({ policy, onDone }: { policy: AdminLibraryPolicy; onDone: () => void }) {
  const [d, setD] = useState({ title: policy.title, summary: policy.summary ?? "", days: String(policy.review_every_days), reason: "" });
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await apiClient.patchLibraryPolicy(policy.id, { title: d.title.trim(), summary: d.summary, review_every_days: Number(d.days), reason: d.reason.trim() });
      onDone();
    } catch (err) {
      setError(err);
    } finally {
      setBusy(false);
    }
  };
  return (
    <form onSubmit={(e) => void submit(e)} className="space-y-2 rounded border border-gray-200 dark:border-gray-800 p-3">
      {error !== null && <ErrorState error={error} />}
      <label htmlFor="edit-title" className="block text-xs font-medium">
        Title
        <input id="edit-title" className={inputClass} value={d.title} onChange={(e) => setD({ ...d, title: e.target.value })} />
      </label>
      <label htmlFor="edit-summary" className="block text-xs font-medium">
        Summary
        <textarea id="edit-summary" rows={2} className={inputClass} value={d.summary} onChange={(e) => setD({ ...d, summary: e.target.value })} />
      </label>
      <label htmlFor="edit-days" className="block text-xs font-medium">
        Review every (days)
        <input id="edit-days" type="number" min={1} max={3650} className={inputClass} value={d.days} onChange={(e) => setD({ ...d, days: e.target.value })} />
      </label>
      <label htmlFor="edit-reason" className="block text-xs font-medium">
        Reason
        <textarea id="edit-reason" rows={2} className={inputClass} value={d.reason} onChange={(e) => setD({ ...d, reason: e.target.value })} />
      </label>
      <button type="submit" className={buttonClass} disabled={busy || !d.reason.trim() || !d.title.trim()}>
        Save changes
      </button>
    </form>
  );
}

const STATUS_ORDER = ["Published", "InReview", "Draft", "Rejected", "Superseded", "Retired"];

function PolicyDetail({ id }: { id: string }) {
  const [page, setPage] = useState<any>(null);
  const [error, setError] = useState<unknown>(null);
  const [editing, setEditing] = useState(false);
  const [comparing, setComparing] = useState<string | null>(null);
  const load = useCallback(async () => {
    setError(null);
    try {
      setPage(await apiClient.getAdminLibraryPolicy(id));
    } catch (e) {
      setError(e);
    }
  }, [id]);
  useEffect(() => {
    void load();
  }, [load]);
  if (error !== null) return <ErrorState error={error} />;
  if (!page) return <Skeleton rows={6} />;
  const p: AdminLibraryPolicy = page.policy;
  const rules: any[] = page.rules ?? [];
  // A source with two snapshots or more can be compared with its earlier copy.
  const copies = new Map<string, number>();
  for (const n of page.snapshots ?? []) copies.set(n.source_id, (copies.get(n.source_id) ?? 0) + 1);
  return (
    <div className="space-y-5">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h2 className="text-base font-semibold">{p.title}</h2>
          <p className="text-xs text-gray-500">
            {`Owner: ${ownerText(p.owner)} · Review every ${p.review_every_days} days · `}
            {p.due ? <span className="text-amber-700 dark:text-amber-300 font-medium">Review due</span> : `Reviewed ${day(p.last_reviewed_at)}`}
          </p>
        </div>
        {!editing && (
          <button type="button" className={buttonClass} onClick={() => setEditing(true)}>
            Edit
          </button>
        )}
      </div>
      {editing && (
        <EditPolicy
          policy={p}
          onDone={() => {
            setEditing(false);
            void load();
          }}
        />
      )}

      <section className="space-y-2">
        <h3 className="text-sm font-semibold">Sources</h3>
        {(page.sources ?? []).length === 0 ? (
          <p className="text-xs text-gray-500">No sources yet.</p>
        ) : (
          <ul className="space-y-2">
            {page.sources.map((s: any) => (
              <li key={s.id} id={`source-${s.id}`} className="rounded border border-gray-200 dark:border-gray-800 p-2 text-sm space-y-1">
                <div className="flex flex-wrap justify-between gap-2">
                  <span className="font-medium">{s.title}</span>
                  <span className="text-xs text-gray-500">{`${s.last_status ?? s.state} · Last checked ${day(s.last_checked_at)}`}</span>
                </div>
                {s.remedy && <p className="text-xs text-amber-700 dark:text-amber-300">{s.remedy}</p>}
                {(copies.get(s.id) ?? 0) >= 2 && (
                  <button type="button" className={buttonClass} onClick={() => setComparing(comparing === s.id ? null : s.id)}>
                    {comparing === s.id ? "Hide comparison" : "Compare with the earlier copy"}
                  </button>
                )}
                {comparing === s.id && <SourceCompare sourceId={s.id} />}
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="space-y-1">
        <h3 className="text-sm font-semibold">{`Snapshots (${(page.snapshots ?? []).length})`}</h3>
        <ul className="text-xs text-gray-600 dark:text-gray-400 space-y-0.5">
          {(page.snapshots ?? []).slice(0, 20).map((n: any) => (
            <li key={n.id}>{`${day(n.taken_at)} · ${n.media_type} · ${n.text_origin === "supplied" ? "supplied by a person" : "fetched"}`}</li>
          ))}
        </ul>
      </section>

      <section className="space-y-2">
        <h3 className="text-sm font-semibold">Rules</h3>
        {STATUS_ORDER.filter((st) => rules.some((r) => r.status === st)).map((st) => (
          <div key={st} className="space-y-1">
            <h4 className="text-xs font-semibold text-gray-600">{st === "InReview" ? "In review" : st}</h4>
            <ul className="space-y-1 text-sm">
              {rules
                .filter((r) => r.status === st)
                .map((r) => (
                  <li key={r.id} id={`rule-${r.id}`}>
                    <span className="font-mono text-xs">{`${r.key} v${r.version}`}</span> {r.statement}
                    {r.source_changed && <span className="ml-2 text-xs text-amber-700 dark:text-amber-300">{`Its source changed on ${day(r.source_changed_at)}.`}</span>}
                  </li>
                ))}
            </ul>
          </div>
        ))}
        {rules.length === 0 && <p className="text-xs text-gray-500">No rules yet.</p>}
      </section>

      <section className="space-y-1">
        <h3 className="text-sm font-semibold">{`Open questions (${(page.open_questions ?? []).length})`}</h3>
        <ul className="text-sm space-y-1">
          {(page.open_questions ?? []).map((q: any) => (
            <li key={q.id}>{`${q.question} · asked ${day(q.asked_at)} by ${q.asked_by}`}</li>
          ))}
        </ul>
      </section>

      <section className="space-y-1">
        <h3 className="text-sm font-semibold">{`Bindings (${(page.bindings ?? []).length})`}</h3>
        <ul className="text-sm space-y-1">
          {(page.bindings ?? []).map((b: any) => (
            <li key={b.id}>{`${b.rule_key} is enforced by ${b.target_kind.replace(/_/g, " ")} ${b.target_ref}`}</li>
          ))}
        </ul>
      </section>
    </div>
  );
}

// The Policy library section of the console (policy library phase 6): the policies, one policy, and the state of the
// library. `/admin/library`, `/admin/library/health`, and `/admin/library/{id}`.
export function PolicyLibrary({ path, navigate }: { path: string; navigate: (to: string) => void }) {
  const rest = path.replace(/^\/admin\/library\/?/, "").split(/[?#]/)[0];
  const view = rest === "" ? "list" : rest === "health" ? "health" : "policy";
  const tab = (to: string, label: string, on: boolean) => (
    <a
      href={to}
      aria-current={on ? "page" : undefined}
      className={`px-3 py-1.5 text-sm rounded ${on ? "bg-gray-200 dark:bg-gray-800 font-semibold" : ""}`}
      onClick={(e) => {
        e.preventDefault();
        navigate(to);
      }}
    >
      {label}
    </a>
  );
  return (
    <div className="space-y-4">
      <PageHeader title="Policy library" description="The institution's policies, the rules read from them, and what keeps each rule true." />
      <nav className="flex gap-2" aria-label="Policy library">
        {tab("/admin/library", "Policies", view === "list")}
        {tab("/admin/library/health", "Health", view === "health")}
      </nav>
      {view === "list" && <PolicyList navigate={navigate} />}
      {view === "health" && <Health navigate={navigate} />}
      {view === "policy" && <PolicyDetail id={rest} />}
    </div>
  );
}
