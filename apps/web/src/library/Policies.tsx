import { useCallback, useEffect, useState } from "react";
import { apiClient, type LibraryPolicyPage, type LibraryRulePage, type LibraryRuleSummary } from "../api";
import { EmptyState, ErrorState, FormField, PageHeader, Skeleton, StatusBadge } from "../ui";

const inputClass = "w-full rounded border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-900 p-2 text-sm";
const linkButton = "text-blue-600 dark:text-blue-400 hover:underline text-left";

type View = { kind: "list" } | { kind: "policy"; key: string } | { kind: "rule"; policy: string; rule: string };

function viewOf(path: string): View {
  const parts = path.replace(/^\/policies\/?/, "").split("/").filter(Boolean);
  if (parts.length >= 2) return { kind: "rule", policy: parts[0], rule: parts[1] };
  if (parts.length === 1) return { kind: "policy", key: parts[0] };
  return { kind: "list" };
}

function date(iso: string | null | undefined): string {
  return iso ? iso.slice(0, 10) : "not yet";
}

/** A rule is a sentence. The policy it belongs to is a link, and a chip says whether anything enforces it. */
function RuleCard({ rule, onPolicy, onRule }: { rule: LibraryRuleSummary; onPolicy: () => void; onRule: () => void }) {
  return (
    <article data-testid={`library-rule-${rule.id}`} className="border border-gray-200 dark:border-gray-800 rounded-lg p-4 space-y-2">
      <button type="button" onClick={onRule} className="text-base text-gray-900 dark:text-gray-100 text-left hover:underline">
        {rule.statement}
      </button>
      <div className="flex flex-wrap items-center gap-3 text-sm">
        <button type="button" onClick={onPolicy} className={linkButton}>
          {rule.policy_title}
        </button>
        <StatusBadge tone={rule.enforcement === "Enforced" ? "ok" : "neutral"} text={rule.enforcement} />
        {rule.applies_to && <span className="text-gray-500">Applies to {rule.applies_to}</span>}
        <span className="text-gray-500">Policy last reviewed {date(rule.last_reviewed_at)}</span>
      </div>
      {rule.enforced_by.length > 0 && (
        <ul className="text-xs text-gray-600 dark:text-gray-400">
          {rule.enforced_by.map((b) => (
            <li key={b.id}>
              {b.state === "Enforced" ? "Enforced by" : "Was enforced by"} {b.name}
              {b.state === "Missing" ? " (no longer there)" : ""}
            </li>
          ))}
        </ul>
      )}
      {rule.warning && <p className="text-xs text-amber-700 dark:text-amber-300">{rule.warning}</p>}
    </article>
  );
}

/** The Policies page: the rules the institution has published, in words, and what enforces each one. */
export function Policies({ path = "/policies", navigate }: { path?: string; navigate?: (to: string) => void }) {
  const [view, setView] = useState<View>(() => viewOf(path));
  const go = (v: View) => {
    setView(v);
    navigate?.(v.kind === "list" ? "/policies" : v.kind === "policy" ? `/policies/${v.key}` : `/policies/${v.policy}/${v.rule}`);
  };

  return (
    <div className="space-y-6" data-testid="policies-page">
      <PageHeader title="Policies" description="The rules your institution has published, and what enforces each one" />
      {view.kind === "list" && <RuleList onPolicy={(key) => go({ kind: "policy", key })} onRule={(r) => go({ kind: "rule", policy: r.policy_key, rule: r.rule_key })} />}
      {view.kind === "policy" && <PolicyView policyKey={view.key} onBack={() => go({ kind: "list" })} onRule={(r) => go({ kind: "rule", policy: r.policy_key, rule: r.rule_key })} />}
      {view.kind === "rule" && <RuleView policyKey={view.policy} ruleKey={view.rule} onBack={() => go({ kind: "list" })} onPolicy={() => go({ kind: "policy", key: view.policy })} />}
    </div>
  );
}

function RuleList({ onPolicy, onRule }: { onPolicy: (key: string) => void; onRule: (r: LibraryRuleSummary) => void }) {
  const [rules, setRules] = useState<LibraryRuleSummary[] | null>(null);
  const [more, setMore] = useState<string | null>(null);
  const [q, setQ] = useState("");
  const [enforcement, setEnforcement] = useState("");
  const [error, setError] = useState<unknown>(null);

  const load = useCallback(async () => {
    try {
      const page = await apiClient.library({ q, enforcement });
      setRules(page.rules);
      setMore(page.next_cursor);
      setError(null);
    } catch (e) {
      setError(e);
    }
  }, [q, enforcement]);
  useEffect(() => {
    void load();
  }, [load]);

  const loadMore = async () => {
    if (!more) return;
    const page = await apiClient.library({ q, enforcement, cursor: more });
    setRules((r) => [...(r ?? []), ...page.rules]);
    setMore(page.next_cursor);
  };

  return (
    <>
      <div className="grid gap-3 md:grid-cols-2">
        <FormField id="library-search" label="Search">
          {(props) => <input {...props} className={inputClass} value={q} onChange={(e) => setQ(e.target.value)} />}
        </FormField>
        <FormField id="library-enforcement" label="Enforcement">
          {(props) => (
            <select {...props} className={inputClass} value={enforcement} onChange={(e) => setEnforcement(e.target.value)}>
              <option value="">All rules</option>
              <option value="Enforced">Enforced</option>
              <option value="Guidance">Guidance</option>
            </select>
          )}
        </FormField>
      </div>
      {error !== null && <ErrorState error={error} onRetry={() => void load()} />}
      {rules === null && error === null ? (
        <Skeleton rows={3} />
      ) : rules !== null && rules.length === 0 ? (
        <EmptyState title="No rules match" description="Only rules a reviewer has published are shown, for the units you belong to and the institution." />
      ) : (
        <div className="space-y-3">
          {rules?.map((r) => (
            <RuleCard key={r.id} rule={r} onPolicy={() => onPolicy(r.policy_key)} onRule={() => onRule(r)} />
          ))}
          {more && (
            <button type="button" onClick={() => void loadMore()} className="text-xs text-blue-600 hover:underline">
              Show more
            </button>
          )}
        </div>
      )}
    </>
  );
}

function PolicyView({ policyKey, onBack, onRule }: { policyKey: string; onBack: () => void; onRule: (r: LibraryRuleSummary) => void }) {
  const [page, setPage] = useState<LibraryPolicyPage | null>(null);
  const [error, setError] = useState<unknown>(null);
  useEffect(() => {
    let live = true;
    apiClient.libraryPolicy(policyKey).then((p) => live && setPage(p)).catch((e) => live && setError(e));
    return () => {
      live = false;
    };
  }, [policyKey]);
  if (error !== null) return <ErrorState error={error} />;
  if (!page) return <Skeleton rows={3} />;
  return (
    <div className="space-y-4">
      <button type="button" onClick={onBack} className={`${linkButton} text-sm`}>
        All policies
      </button>
      <h2 className="text-lg font-semibold text-gray-900 dark:text-gray-100">{page.policy.title}</h2>
      {page.policy.summary && <p className="text-sm text-gray-700 dark:text-gray-300">{page.policy.summary}</p>}
      <p className="text-xs text-gray-500">Last reviewed {date(page.policy.last_reviewed_at)}</p>
      <section aria-label="Rules" className="space-y-3">
        {page.rules.map((r) => (
          <RuleCard key={r.id} rule={r} onPolicy={() => {}} onRule={() => onRule(r)} />
        ))}
      </section>
      <section aria-label="Sources" className="space-y-1">
        <h3 className="text-sm font-semibold text-gray-800 dark:text-gray-200">Sources</h3>
        {page.sources.map((s) => (
          <div key={`${s.title}-${s.href ?? ""}`} className="text-sm">
            {s.href ? (
              <a href={s.href} target="_blank" rel="noopener noreferrer" className="text-blue-600 dark:text-blue-400 hover:underline">
                {s.title}
              </a>
            ) : (
              <span className="text-gray-900 dark:text-gray-100">{s.title}</span>
            )}
            <span className="text-xs text-gray-500"> Last checked {date(s.last_checked_at)}</span>
          </div>
        ))}
      </section>
    </div>
  );
}

function RuleView({ policyKey, ruleKey, onBack, onPolicy }: { policyKey: string; ruleKey: string; onBack: () => void; onPolicy: () => void }) {
  const [page, setPage] = useState<LibraryRulePage | null>(null);
  const [error, setError] = useState<unknown>(null);
  useEffect(() => {
    let live = true;
    apiClient.libraryRule(policyKey, ruleKey).then((p) => live && setPage(p)).catch((e) => live && setError(e));
    return () => {
      live = false;
    };
  }, [policyKey, ruleKey]);
  if (error !== null) return <ErrorState error={error} />;
  if (!page) return <Skeleton rows={3} />;
  return (
    <div className="space-y-4">
      <button type="button" onClick={onBack} className={`${linkButton} text-sm`}>
        All policies
      </button>
      <RuleCard rule={page.rule} onPolicy={onPolicy} onRule={() => {}} />
      <section aria-label="Where it comes from" className="space-y-2">
        <h3 className="text-sm font-semibold text-gray-800 dark:text-gray-200">Where it comes from</h3>
        {page.citations.map((c, i) => (
          <blockquote key={i} className="border border-gray-200 dark:border-gray-800 rounded-lg p-3 text-sm whitespace-pre-wrap text-gray-700 dark:text-gray-300">
            {c.before}
            <mark className="bg-yellow-200 dark:bg-yellow-700 text-inherit">{c.passage}</mark>
            {c.after}
            <footer className="mt-1 text-xs text-gray-500">{c.source_title ?? "Source"}</footer>
          </blockquote>
        ))}
      </section>
      <section aria-label="History" className="space-y-1">
        <h3 className="text-sm font-semibold text-gray-800 dark:text-gray-200">Earlier wording</h3>
        {page.history.map((h) => (
          <div key={h.version} className="text-sm text-gray-700 dark:text-gray-300">
            {h.statement} <span className="text-xs text-gray-500">Approved by {h.decided_by ?? "unknown"} on {date(h.decided_at)}</span>
          </div>
        ))}
      </section>
    </div>
  );
}
