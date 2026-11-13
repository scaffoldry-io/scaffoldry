import { useMemo, useState, type FormEvent } from "react";
import { apiClient, type AppView } from "../api";
import { visibleFields, type Condition } from "../showWhen";

/** What a form needs to know of a field of the table: its label, its type, whether it is required, and its options. */
export interface FieldMeta {
  name: string;
  label: string;
  field_type: string;
  required?: boolean;
  select_options?: { id: string; label: string }[];
}

interface FormEntry {
  name: string;
  label?: string;
  help?: string;
  show_when?: Condition[];
}

const inputClass = "w-full rounded border border-gray-300 dark:border-gray-700 p-2 text-sm bg-white dark:bg-gray-800";

const isEmpty = (v: unknown) => v === undefined || v === null || (typeof v === "string" && v.trim() === "") || (Array.isArray(v) && v.length === 0);

/**
 * A form view: some fields of a table, in order, each with a label and help, shown while its conditions hold. The conditions
 * run here for immediate feedback, and again on the server at submit, which is authoritative. A field the conditions hide is
 * not required and is not sent.
 */
export function FormView({ appSlug, view, fields }: { appSlug: string; view: AppView; fields: FieldMeta[] }) {
  const entries: FormEntry[] = useMemo(() => (Array.isArray(view.config?.form_fields) ? (view.config.form_fields as FormEntry[]) : []), [view]);
  const [answers, setAnswers] = useState<Record<string, unknown>>({});
  const [problem, setProblem] = useState<string | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const shown = visibleFields(entries, answers);
  const meta = (name: string) => fields.find((f) => f.name === name);
  const labelOf = (e: FormEntry) => e.label ?? meta(e.name)?.label ?? e.name;

  const set = (name: string, value: unknown) => setAnswers((a) => ({ ...a, [name]: value }));

  const submit = async (ev: FormEvent) => {
    ev.preventDefault();
    setProblem(null);
    // What is sent is what is shown, and only that: an answer given before a condition hid its field is not kept.
    const data: Record<string, unknown> = {};
    for (const [i, e] of entries.entries()) {
      if (!shown[i]) continue;
      const value = answers[e.name];
      if (isEmpty(value)) {
        if (meta(e.name)?.required) {
          setProblem(`${labelOf(e)} is required.`);
          return;
        }
        continue;
      }
      data[e.name] = value;
    }
    setBusy(true);
    try {
      const res = await apiClient.submitForm(appSlug, view.id, data);
      setDone(res.success_message);
    } catch (e) {
      setProblem(e instanceof Error ? e.message : "The form could not be sent.");
    } finally {
      setBusy(false);
    }
  };

  if (done !== null) return <p role="status" className="rounded border p-4 text-sm">{done}</p>;

  return (
    <form onSubmit={submit} className="space-y-4" aria-label={view.name}>
      {entries.map((e, i) => {
        if (!shown[i]) return null;
        const m = meta(e.name);
        const id = `${view.id}-${e.name}`;
        const required = m?.required === true;
        const type = m?.field_type ?? "Text";
        return (
          <div key={e.name} className="space-y-1">
            <label htmlFor={id} className="block text-sm font-medium">
              {labelOf(e)}
              {required && <span aria-hidden="true"> *</span>}
            </label>
            {type === "Select" ? (
              <select id={id} className={inputClass} value={String(answers[e.name] ?? "")} onChange={(ev) => set(e.name, ev.target.value)}>
                <option value="">Choose one</option>
                {(m?.select_options ?? []).map((o) => (
                  <option key={o.id} value={o.id}>{o.label}</option>
                ))}
              </select>
            ) : type === "Boolean" ? (
              <input id={id} type="checkbox" checked={answers[e.name] === true} onChange={(ev) => set(e.name, ev.target.checked)} />
            ) : type === "Number" ? (
              <input id={id} type="number" className={inputClass} value={String(answers[e.name] ?? "")} onChange={(ev) => set(e.name, ev.target.value === "" ? undefined : Number(ev.target.value))} />
            ) : type === "Date" ? (
              <input id={id} type="date" className={inputClass} value={String(answers[e.name] ?? "")} onChange={(ev) => set(e.name, ev.target.value)} />
            ) : (
              <input id={id} type="text" className={inputClass} value={String(answers[e.name] ?? "")} onChange={(ev) => set(e.name, ev.target.value)} />
            )}
            {e.help && <p className="text-xs text-gray-600 dark:text-gray-400">{e.help}</p>}
          </div>
        );
      })}
      {problem && <p role="alert" className="text-sm text-red-700">{problem}</p>}
      <button type="submit" disabled={busy} className="rounded border px-3 py-1.5 text-sm font-medium disabled:opacity-50">
        Submit
      </button>
    </form>
  );
}
