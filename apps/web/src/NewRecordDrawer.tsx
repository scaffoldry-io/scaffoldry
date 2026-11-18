import React, { useState } from "react";
import { ApiError } from "./api";
import { Drawer } from "./ui/Drawer";
import { FormField } from "./ui/FormField";
import { explainError } from "./ui/explainError";
import { normalizeSelectOption, type FieldSpec } from "./types";

/** Types the server fills in or works out. A person does not type them. */
const SYSTEM_TYPES = new Set([
  "Autonumber", "CreatedTime", "LastModifiedTime", "CreatedBy", "LastModifiedBy",
  "Lookup", "Count", "Rollup", "Formula", "Button", "Attachment",
]);

/** The words for a refused save: the server's own sentence when it sent one, else the usual explanation. */
export function sayStoreError(err: unknown): string {
  if (err instanceof ApiError && err.status > 0 && err.message && !/^HTTP \d/.test(err.message)) {
    return [err.message, err.remedy].filter(Boolean).join(" ");
  }
  const x = explainError(err);
  return [x.message, x.next].filter(Boolean).join(" ");
}

/** The fields a person fills in when adding a record. */
export function typedFields(fields: FieldSpec[]): FieldSpec[] {
  return fields.filter((f) => f.name !== "id" && f.name !== "_table_id" && !SYSTEM_TYPES.has(f.field_type));
}

/** The fields a new record cannot be stored without. */
export function requiredFields(fields: FieldSpec[]): FieldSpec[] {
  return typedFields(fields).filter((f) => f.required);
}

const NUMERIC = new Set(["Number", "Currency", "Percent", "Rating"]);
const TOGGLE = new Set(["Boolean", "Checkbox"]);
const INPUT = "w-full px-3 py-2 rounded-lg border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-900 text-sm text-gray-900 dark:text-gray-100";

interface Props {
  open: boolean;
  onClose: () => void;
  /** The fields to ask for. */
  fields: FieldSpec[];
  /** Stores the record. A rejection is shown in the form. */
  onSubmit: (values: Record<string, unknown>) => Promise<unknown>;
}

/** A small form that asks for a new record's required fields, then stores it. The server's refusal is shown here. */
export const NewRecordDrawer: React.FC<Props> = ({ open, onClose, fields, onSubmit }) => {
  const [values, setValues] = useState<Record<string, unknown>>({});
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});
  const [formError, setFormError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const set = (name: string, value: unknown) => {
    setValues((prev) => ({ ...prev, [name]: value }));
    setFieldErrors((prev) => {
      if (!prev[name]) return prev;
      const next = { ...prev };
      delete next[name];
      return next;
    });
  };

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setSaving(true);
    setFormError(null);
    setFieldErrors({});
    // A field left alone is sent as empty, so the server names the one it needs.
    const data = Object.fromEntries(
      fields.map((f) => [f.name, values[f.name] ?? (TOGGLE.has(f.field_type) ? false : "")])
    );
    try {
      await onSubmit(data);
      setValues({});
      onClose();
    } catch (err) {
      const named = err instanceof ApiError && err.details && typeof err.details.field === "string" ? (err.details.field as string) : null;
      const text = sayStoreError(err);
      const bad = err instanceof ApiError && err.fields ? err.fields : named ? { [named]: text } : null;
      if (bad && Object.keys(bad).length > 0) setFieldErrors(bad);
      else setFormError(text);
    } finally {
      setSaving(false);
    }
  };

  return (
    <Drawer open={open} onClose={onClose} title="New record" testId="new-record-drawer">
      <form onSubmit={submit} className="space-y-4">
        {formError && <p role="alert" className="text-sm font-medium text-red-600 dark:text-red-400">{formError}</p>}
        {fields.map((f) => {
          const id = `new-record-${f.name}`;
          const v = values[f.name];
          return (
            <FormField key={f.name} id={id} label={f.label} required={f.required} error={fieldErrors[f.name]}>
              {(p) => {
                if (TOGGLE.has(f.field_type)) {
                  return <input {...p} type="checkbox" checked={v === true} onChange={(e) => set(f.name, e.target.checked)} />;
                }
                if (f.field_type === "Select") {
                  const options = (f.select_options ?? []).map((o, i) => normalizeSelectOption(o, i));
                  return (
                    <select {...p} className={INPUT} value={(v as string) ?? ""} onChange={(e) => set(f.name, e.target.value)}>
                      <option value="">Choose one</option>
                      {options.map((o) => <option key={o.id} value={o.id}>{o.label}</option>)}
                    </select>
                  );
                }
                if (f.field_type === "LongText") {
                  return <textarea {...p} className={INPUT} rows={4} value={(v as string) ?? ""} onChange={(e) => set(f.name, e.target.value)} />;
                }
                const type = NUMERIC.has(f.field_type) ? "number" : f.field_type === "Date" ? "date" : f.field_type === "DateTime" ? "datetime-local" : "text";
                return (
                  <input
                    {...p}
                    type={type}
                    className={INPUT}
                    value={(v as string | number) ?? ""}
                    onChange={(e) => set(f.name, type === "number" && e.target.value !== "" ? Number(e.target.value) : e.target.value)}
                  />
                );
              }}
            </FormField>
          );
        })}
        <div className="flex justify-end gap-2 pt-2">
          <button type="button" onClick={onClose} className="px-3 py-2 rounded-lg border border-gray-300 dark:border-gray-700 text-sm cursor-pointer">
            Cancel
          </button>
          <button type="submit" disabled={saving} className="px-4 py-2 rounded-lg bg-blue-600 hover:bg-blue-700 text-white text-sm font-semibold cursor-pointer disabled:opacity-60">
            {saving ? "Saving" : "Save record"}
          </button>
        </div>
      </form>
    </Drawer>
  );
};
