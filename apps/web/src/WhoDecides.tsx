import { useEffect, useState } from "react";
import { apiClient, type ApproverPreview } from "./api";
import type { ApproverSpec } from "./types";
import { Banner } from "./ui";

type Kind = "position" | "person" | "role";

/** The words for a preview: who it would go to today, or why nobody. */
export function previewWords(p: ApproverPreview, spec: ApproverSpec, positions: { key: string; name: string }[]): { ok: boolean; text: string } {
  if (p.problem) {
    const start = p.start_unit_name ?? "the starting unit";
    const walk = "Position" in spec && spec.Position.walk_up;
    const text = {
      position_vacant: walk ? `This position is vacant at ${start} and above.` : `This position is vacant at ${start}.`,
      only_submitter: "Only the person who submitted the record would decide, and nobody decides their own record.",
      person_unavailable: "That person is not active or is on hold, so they cannot decide.",
      role_empty: "Nobody in the workspace has that role.",
    }[p.problem];
    return { ok: false, text };
  }
  if (p.approvers.length === 0) return { ok: false, text: "Nobody would decide this today." };
  const position = "Position" in spec ? positions.find((x) => x.key === spec.Position.key)?.name ?? spec.Position.key : null;
  const names = p.approvers.map((a) => {
    if (typeof a.via === "object" && "DelegateOf" in a.via) return `${a.display_name}, deciding for ${a.via.DelegateOf}`;
    if (a.via === "Holder" && position) return p.unit_name ? `${a.display_name}, ${position} of ${p.unit_name}` : `${a.display_name}, ${position}`;
    return a.display_name;
  });
  return { ok: true, text: `Today this would go to: ${names.join("; ")}.` };
}

/**
 * The `Who decides` control of a user task (approvers phase 5): a position from a starting unit, a specific person, or
 * a workspace role, with a live preview of who that is today.
 */
export function WhoDecides({ appSlug, onChange }: { appSlug: string; onChange: (spec: ApproverSpec | null) => void }) {
  const [kind, setKind] = useState<Kind>("position");
  const [positions, setPositions] = useState<{ key: string; name: string }[]>([]);
  const [key, setKey] = useState("");
  const [from, setFrom] = useState<"Submitter" | "Workspace">("Submitter");
  const [walkUp, setWalkUp] = useState(true);
  const [personQuery, setPersonQuery] = useState("");
  const [people, setPeople] = useState<{ eppn: string; name: string }[]>([]);
  const [person, setPerson] = useState("");
  const [role, setRole] = useState("");
  const [preview, setPreview] = useState<{ ok: boolean; text: string } | null>(null);

  useEffect(() => {
    apiClient.approverOptions(appSlug).then(setPositions).catch(() => setPositions([]));
  }, [appSlug]);

  const spec: ApproverSpec | null =
    kind === "position"
      ? key
        ? { Position: { key, from, walk_up: walkUp } }
        : null
      : kind === "person"
        ? person
          ? { Person: { eppn: person } }
          : null
        : role.trim()
          ? { Role: { name: role.trim() } }
          : null;
  const specKey = JSON.stringify(spec);

  useEffect(() => {
    onChange(spec);
    if (!spec) {
      setPreview(null);
      return;
    }
    let alive = true;
    apiClient
      .previewApprovers(appSlug, { approver: spec })
      .then((p) => alive && setPreview(previewWords(p, spec, positions)))
      .catch(() => alive && setPreview(null));
    return () => {
      alive = false;
    };
    // The spec is compared by its JSON, so a new object with the same content does not ask again.
  }, [specKey, appSlug, positions]);

  useEffect(() => {
    if (kind !== "person" || personQuery.trim().length < 2) {
      setPeople([]);
      return;
    }
    let alive = true;
    apiClient.searchDelegates(personQuery.trim()).then((l) => alive && setPeople(l)).catch(() => alive && setPeople([]));
    return () => {
      alive = false;
    };
  }, [kind, personQuery]);

  const input = "w-full px-2.5 py-1 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white";
  const choice = (k: Kind, label: string) => (
    <label className="flex items-center gap-1.5 cursor-pointer">
      <input type="radio" name="who-decides" checked={kind === k} onChange={() => setKind(k)} />
      <span>{label}</span>
    </label>
  );
  return (
    <fieldset className="space-y-2 text-xs">
      <legend className="text-[11px] font-bold uppercase tracking-wider text-slate-500">Who decides</legend>
      <div className="flex flex-wrap gap-4">
        {choice("position", "A position")}
        {choice("person", "A specific person")}
        {choice("role", "A workspace role (the old behaviour)")}
      </div>
      {kind === "position" && (
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
          <label className="block">
            <span className="block text-[11px] text-slate-500 mb-1">Position</span>
            <select aria-label="Position" value={key} onChange={(e) => setKey(e.target.value)} className={input}>
              <option value="">Choose a position</option>
              {positions.map((p) => (
                <option key={p.key} value={p.key}>
                  {p.name}
                </option>
              ))}
            </select>
          </label>
          <label className="block">
            <span className="block text-[11px] text-slate-500 mb-1">Starting from</span>
            <select aria-label="Starting from" value={from} onChange={(e) => setFrom(e.target.value as "Submitter" | "Workspace")} className={input}>
              <option value="Submitter">the person who submitted the record</option>
              <option value="Workspace">the workspace&apos;s unit</option>
            </select>
          </label>
          <label className="flex items-center gap-2 sm:col-span-2 cursor-pointer">
            <input type="checkbox" checked={walkUp} onChange={(e) => setWalkUp(e.target.checked)} />
            <span>If the position is vacant, ask the next unit up</span>
          </label>
        </div>
      )}
      {kind === "person" && (
        <label className="block">
          <span className="block text-[11px] text-slate-500 mb-1">Person</span>
          <input aria-label="Person" type="text" value={personQuery} onChange={(e) => setPersonQuery(e.target.value)} placeholder="Type a name" className={input} />
          {people.length > 0 && (
            <ul role="listbox" aria-label="People found" className="mt-1 rounded border border-slate-200 dark:border-slate-700">
              {people.map((p) => (
                <li
                  key={p.eppn}
                  role="option"
                  aria-selected={person === p.eppn}
                  tabIndex={0}
                  onClick={() => {
                    setPerson(p.eppn);
                    setPersonQuery(`${p.name} <${p.eppn}>`);
                    setPeople([]);
                  }}
                  className="cursor-pointer px-2 py-1 hover:bg-slate-100 dark:hover:bg-slate-800"
                >
                  {p.name} <span className="text-slate-500">{p.eppn}</span>
                </li>
              ))}
            </ul>
          )}
        </label>
      )}
      {kind === "role" && (
        <label className="block">
          <span className="block text-[11px] text-slate-500 mb-1">Role name</span>
          <input aria-label="Role name" type="text" value={role} onChange={(e) => setRole(e.target.value)} className={input} />
        </label>
      )}
      {preview && (preview.ok ? <p className="text-slate-700 dark:text-slate-300">{preview.text}</p> : <Banner kind="warning">{preview.text}</Banner>)}
    </fieldset>
  );
}
