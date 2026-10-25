import { useEffect, useState } from "react";
import { apiClient, type AccessExplainGrant, type AccessExplanation } from "../api";
import { ErrorState } from "../ui";
import { grantSentence } from "./accessSentences";

const inputClass =
  "w-full text-sm rounded border border-gray-300 dark:border-gray-700 p-2 bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100";

type Person = { eppn: string; name: string };

/** "person:dana@state.edu" is Dana. "position:chair@*" is every chair. A unit or a group is named as it is. */
export function holderWords(holder: string): string {
  const [kind, ...rest] = holder.split(":");
  const id = rest.join(":");
  if (kind === "person") return id;
  if (kind === "position") return `Every ${id.split("@")[0]}`;
  return holder;
}

function axisWords(axis: string | undefined): string {
  if (!axis) return "it does not reach it.";
  if (axis.startsWith("unit:")) return `the object sits in a different unit of the ${axis.slice(5)} hierarchy.`;
  if (axis === "domain") return "the object sits in a different domain.";
  if (axis === "sealed") return "a sealed container stops it.";
  if (axis === "time") return "it is not in force now.";
  if (axis === "holder") return "the holder is not the person.";
  return `${axis}.`;
}

export function ExplainedGrant({ g, reached }: { g: AccessExplainGrant; reached: boolean }) {
  const sentence = grantSentence({
    holder: holderWords(g.holder),
    role: g.role,
    anchor: g.anchor_name ?? g.anchor,
    anchorKind: g.anchor.startsWith("unit:") || g.anchor === "holder_unit" ? (g.anchor === "holder_unit" ? "holder_unit" : "unit") : "container",
    depth: g.depth,
    domain: g.domain,
    clearance: g.clearance ?? null,
  });
  const id = g.grant_id;
  const link = id ? (
    <a className="text-blue-600 hover:underline text-xs" href={`/admin/access/grants/${id}`}>
      Grant {id}
    </a>
  ) : (
    <span className="text-xs text-gray-500">From {g.source === "stored" ? "a grant" : g.source}</span>
  );
  return (
    <li data-testid={`explain-${reached ? "reached" : "missed"}-${id ?? g.source}`} className="border border-gray-200 dark:border-gray-800 rounded-lg p-3 text-sm space-y-1">
      <div className="text-gray-900 dark:text-gray-100">{sentence}</div>
      {reached ? (
        g.path_names && g.path_names.length > 0 && <div className="text-gray-600 dark:text-gray-400">{g.path_names.join(" › ")}</div>
      ) : (
        <div className="text-gray-600 dark:text-gray-400">This grant does not reach this object: {axisWords(g.axis)}</div>
      )}
      {link}
    </li>
  );
}

export function AccessPeople() {
  const [q, setQ] = useState("");
  const [people, setPeople] = useState<Person[]>([]);
  const [more, setMore] = useState<string | null>(null);
  const [chosen, setChosen] = useState<Person | null>(null);
  const [object, setObject] = useState("");
  const [result, setResult] = useState<AccessExplanation | null>(null);
  const [summary, setSummary] = useState<{ grants: number } | null>(null);
  const [error, setError] = useState<unknown>(null);

  useEffect(() => {
    if (q.trim().length < 2) {
      setPeople([]);
      setMore(null);
      return;
    }
    let live = true;
    const t = setTimeout(() => {
      apiClient
        .accessSearchPeople(q.trim())
        .then((r) => {
          if (!live) return;
          setPeople(r.people);
          setMore(r.next_cursor);
        })
        .catch((e) => live && setError(e));
    }, 150);
    return () => {
      live = false;
      clearTimeout(t);
    };
  }, [q]);

  useEffect(() => {
    if (!chosen) return;
    let live = true;
    setResult(null);
    apiClient
      .accessReviewPerson(chosen.eppn)
      .then((r) => live && setSummary({ grants: r.grants.length }))
      .catch(() => live && setSummary(null));
    return () => {
      live = false;
    };
  }, [chosen]);

  const loadMore = async () => {
    if (!more) return;
    const r = await apiClient.accessSearchPeople(q.trim(), more);
    setPeople((p) => [...p, ...r.people]);
    setMore(r.next_cursor);
  };

  const explain = async () => {
    if (!chosen || !object.trim()) return;
    setError(null);
    try {
      setResult(await apiClient.accessExplain(chosen.eppn, object.trim()));
    } catch (e) {
      setResult(null);
      setError(e);
    }
  };

  return (
    <div className="space-y-6" data-testid="access-people">
      <label className="block text-sm font-medium text-gray-700 dark:text-gray-300">
        Search people
        <input type="search" className={`${inputClass} mt-1 max-w-sm`} value={q} onChange={(e) => setQ(e.target.value)} placeholder="A name or an address" />
      </label>
      {people.length > 0 && (
        <ul className="space-y-1">
          {people.map((p) => (
            <li key={p.eppn}>
              <button type="button" onClick={() => setChosen(p)} className="text-left text-sm px-2 py-1 rounded hover:bg-gray-100 dark:hover:bg-gray-800">
                {p.name} <span className="text-gray-500">{p.eppn}</span>
              </button>
            </li>
          ))}
          {more && (
            <li>
              <button type="button" onClick={() => void loadMore()} className="text-xs text-blue-600 hover:underline">
                Show more
              </button>
            </li>
          )}
        </ul>
      )}
      {error !== null && <ErrorState error={error} />}
      {chosen && (
        <section className="space-y-4" aria-label={`Access of ${chosen.name}`}>
          <h3 className="text-sm font-semibold text-gray-900 dark:text-gray-100">
            {chosen.name} <span className="font-normal text-gray-500">{chosen.eppn}</span>
          </h3>
          {summary && <p className="text-sm text-gray-600 dark:text-gray-400">{summary.grants} {summary.grants === 1 ? "grant" : "grants"} held.</p>}
          <div className="flex items-end gap-3">
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 flex-1 max-w-md">
              Object
              <input className={`${inputClass} mt-1`} value={object} onChange={(e) => setObject(e.target.value)} placeholder="workspace:<id>, app:<slug> or table:<slug>/<table id>" />
            </label>
            <button type="button" onClick={() => void explain()} disabled={!object.trim()} className="px-3 py-2 text-sm font-medium rounded-md bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-50">
              Explain
            </button>
          </div>
          {result && (
            <div className="space-y-3">
              <p className="text-sm text-gray-900 dark:text-gray-100">
                {result.role ? `${chosen.name} holds the ${result.role} role on this object.` : `${chosen.name} has no role on this object.`}
                {result.platform_admin ? " They are a Platform Admin." : ""}
              </p>
              {result.reached.length > 0 && (
                <ul className="space-y-2" aria-label="Grants that reach it">
                  {result.reached.map((g, i) => (
                    <ExplainedGrant key={`r${g.grant_id ?? i}`} g={g} reached />
                  ))}
                </ul>
              )}
              {result.missed.length > 0 && (
                <ul className="space-y-2" aria-label="Grants that do not reach it">
                  {result.missed.map((g, i) => (
                    <ExplainedGrant key={`m${g.grant_id ?? i}`} g={g} reached={false} />
                  ))}
                </ul>
              )}
            </div>
          )}
        </section>
      )}
    </div>
  );
}
