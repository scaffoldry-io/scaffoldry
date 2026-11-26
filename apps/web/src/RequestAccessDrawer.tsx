import React, { useState } from "react";
import { apiClient, ApiError, type AccessRequest, type DatasetAccessRequestBody } from "./api";
import { Banner, Drawer, ErrorState, FormField } from "./ui";
import type { CatalogEntry } from "./types";

type FormKey = "reason" | "project" | "duration" | "agreement_reference";

/** The fixed list a dataset's form is chosen from, with the words a person sees. */
const FIELDS: Record<FormKey, { label: string; hint: string }> = {
  reason: { label: "Reason", hint: "Why you need this data. The steward reads it." },
  project: { label: "Project", hint: "The project or study it is for." },
  duration: { label: "Days of access", hint: "A whole number of days. The access ends by itself." },
  agreement_reference: { label: "Agreement reference", hint: "The data use agreement or contract number." },
};
const ORDER: FormKey[] = ["reason", "project", "duration", "agreement_reference"];

const input = "w-full rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-900 p-2 text-sm";
const button = "px-3 py-1.5 rounded-lg text-xs font-semibold";

/** Where the request is, in a sentence the requester can read. */
export function whereIsIt(r: AccessRequest): string {
  switch (r.status) {
    case "Approved":
      return r.ends_at ? `Approved. You have access until ${r.ends_at.slice(0, 10)}.` : "Approved.";
    case "Rejected":
      return r.decision_note ? `Refused. ${r.decision_note}` : "Refused.";
    case "Pending":
      return r.routing?.sentence ?? "It waits for a decision.";
    case "Waiting for person":
      return "It waits for you to confirm it.";
    default:
      return r.status;
  }
}

interface Props {
  dataset: CatalogEntry;
  open: boolean;
  onClose: () => void;
}

/** The form a dataset asks for, and then where the request is. The fields are a setting of the dataset. */
export const RequestAccessDrawer: React.FC<Props> = ({ dataset, open, onClose }) => {
  const listed = ORDER.filter((k) => (dataset.request_fields ?? ["reason"]).includes(k));
  const [values, setValues] = useState<Record<FormKey, string>>({ reason: "", project: "", duration: "", agreement_reference: "" });
  const [problems, setProblems] = useState<Partial<Record<FormKey, string>>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [filed, setFiled] = useState<AccessRequest | null>(null);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    const missing: Partial<Record<FormKey, string>> = {};
    for (const k of listed) {
      if (values[k].trim() === "") missing[k] = `${FIELDS[k].label} is required`;
    }
    if (listed.includes("duration") && values.duration.trim() !== "" && !/^[1-9][0-9]*$/.test(values.duration.trim())) {
      missing.duration = "Days is a whole number";
    }
    setProblems(missing);
    if (Object.keys(missing).length > 0) return;
    const body: DatasetAccessRequestBody = { object_kind: "dataset", object_id: dataset.dataset_id ?? dataset.id, role: "viewer" };
    if (listed.includes("reason")) body.reason = values.reason.trim();
    if (listed.includes("project")) body.project = values.project.trim();
    if (listed.includes("duration")) body.days = Number(values.duration.trim());
    if (listed.includes("agreement_reference")) body.agreement_reference = values.agreement_reference.trim();
    setBusy(true);
    setError(null);
    try {
      setFiled(await apiClient.fileAccessRequest(body));
    } catch (err) {
      // A bad request names the fields. Show each by its field, and the rest as the error.
      if (err instanceof ApiError && err.fields) {
        const named: Partial<Record<FormKey, string>> = {};
        for (const [name, why] of Object.entries(err.fields)) {
          if ((ORDER as string[]).includes(name)) named[name as FormKey] = why;
        }
        setProblems(named);
        if (Object.keys(named).length === Object.keys(err.fields).length) {
          setBusy(false);
          return;
        }
      }
      setError(err);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Drawer open={open} onClose={onClose} title={`Request access to ${dataset.name}`} variant="modal" size="md" testId="request-access">
      {filed ? (
        <div className="space-y-3 text-sm">
          <Banner kind={filed.status === "Rejected" ? "error" : "info"} label="Where your request is">
            <p className="font-semibold">{filed.status}</p>
            <p>{whereIsIt(filed)}</p>
          </Banner>
          {filed.routing?.fallback && <p className="text-xs text-slate-500 dark:text-slate-400">This dataset has no steward to ask, so the usual access rules decided who sees it.</p>}
          <p className="text-xs text-slate-500 dark:text-slate-400">You can follow it, or withdraw it, under your access requests.</p>
          <button type="button" className={`${button} bg-blue-600 text-white`} onClick={onClose}>
            Close
          </button>
        </div>
      ) : (
        <form onSubmit={submit} noValidate aria-label="Request access" className="space-y-4">
          <p className="text-xs text-slate-500 dark:text-slate-400">
            The steward of this dataset decides. You will see where your request is, and who has it. Access you are given ends by itself.
          </p>
          {listed.map((k) => (
            <FormField key={k} id={`request-${k}`} label={FIELDS[k].label} hint={FIELDS[k].hint} error={problems[k]} required>
              {(props) =>
                k === "reason" ? (
                  <textarea {...props} rows={3} maxLength={500} className={input} value={values[k]} onChange={(e) => setValues({ ...values, [k]: e.target.value })} />
                ) : (
                  <input {...props} inputMode={k === "duration" ? "numeric" : undefined} className={input} value={values[k]} onChange={(e) => setValues({ ...values, [k]: e.target.value })} />
                )
              }
            </FormField>
          ))}
          {listed.length === 0 && <p className="text-sm">This dataset asks for nothing more. Send the request.</p>}
          {error !== null && <ErrorState error={error} />}
          <div className="flex gap-2">
            <button type="submit" disabled={busy} className={`${button} bg-blue-600 hover:bg-blue-500 text-white disabled:opacity-60`}>
              {busy ? "Sending…" : "Send request"}
            </button>
            <button type="button" onClick={onClose} className={`${button} bg-slate-100 dark:bg-slate-800 text-slate-700 dark:text-slate-200`}>
              Cancel
            </button>
          </div>
        </form>
      )}
    </Drawer>
  );
};
