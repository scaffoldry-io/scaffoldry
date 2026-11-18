import React, { useState } from "react";
import { Drawer } from "./ui/Drawer";
import { FormField } from "./ui/FormField";
import type { FieldSpec, FieldType } from "./types";

/** The plain field types a person can add. Each stores one value the grid can show and edit. */
export const PLAIN_FIELD_TYPES: { value: FieldType; label: string }[] = [
  { value: "Text", label: "Text" },
  { value: "LongText", label: "Long text" },
  { value: "Number", label: "Number" },
  { value: "Currency", label: "Currency" },
  { value: "Percent", label: "Percent" },
  { value: "Date", label: "Date" },
  { value: "DateTime", label: "Date and time" },
  { value: "Select", label: "Choice from a list" },
  { value: "Checkbox", label: "Checkbox" },
  { value: "Email", label: "Email" },
  { value: "Phone", label: "Phone" },
  { value: "Url", label: "Web address" },
];

/** A field name from its label: lower case, words joined by underscores, and not one the table already has. */
export function fieldNameFor(label: string, taken: string[]): string {
  const base = label.trim().toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_+|_+$/g, "") || "field";
  let name = base;
  for (let n = 2; taken.includes(name); n++) name = `${base}_${n}`;
  return name;
}

interface Props {
  open: boolean;
  onClose: () => void;
  /** The names of the fields the table has. */
  existingNames: string[];
  onAdd: (field: FieldSpec) => void;
  /** Opens the linked field form instead. */
  onAddLinked: () => void;
}

const INPUT = "w-full px-3 py-2 rounded-lg border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-900 text-sm text-gray-900 dark:text-gray-100";

/** A small form that adds a plain field to the open table. */
export const AddFieldDrawer: React.FC<Props> = ({ open, onClose, existingNames, onAdd, onAddLinked }) => {
  const [label, setLabel] = useState("");
  const [type, setType] = useState<FieldType>("Text");
  const [required, setRequired] = useState(false);
  const [choices, setChoices] = useState("");
  const [error, setError] = useState<string | undefined>();

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!label.trim()) {
      setError("Give the field a label.");
      return;
    }
    const options = choices.split(",").map((c) => c.trim()).filter(Boolean);
    if (type === "Select" && options.length === 0) {
      setError("List at least one choice, separated by commas.");
      return;
    }
    onAdd({
      name: fieldNameFor(label, existingNames),
      label: label.trim(),
      field_type: type,
      required,
      ferpa_sensitive: false,
      ...(type === "Select" ? { select_options: options.map((o, i) => ({ id: o, label: o, order: i })) } : {}),
    } as FieldSpec);
    setLabel("");
    setType("Text");
    setRequired(false);
    setChoices("");
    setError(undefined);
    onClose();
  };

  return (
    <Drawer open={open} onClose={onClose} title="Add field" testId="add-field-drawer">
      <form onSubmit={submit} className="space-y-4">
        <FormField id="add-field-label" label="Label" error={error}>
          {(p) => <input {...p} type="text" className={INPUT} value={label} onChange={(e) => { setLabel(e.target.value); setError(undefined); }} />}
        </FormField>
        <FormField id="add-field-type" label="Type">
          {(p) => (
            <select {...p} className={INPUT} value={type} onChange={(e) => setType(e.target.value as FieldType)}>
              {PLAIN_FIELD_TYPES.map((t) => <option key={t.value} value={t.value}>{t.label}</option>)}
            </select>
          )}
        </FormField>
        {type === "Select" && (
          <FormField id="add-field-choices" label="Choices" hint="Separate the choices with commas.">
            {(p) => <input {...p} type="text" className={INPUT} value={choices} onChange={(e) => setChoices(e.target.value)} />}
          </FormField>
        )}
        <label className="flex items-center gap-2 text-sm text-gray-700 dark:text-gray-300">
          <input type="checkbox" checked={required} onChange={(e) => setRequired(e.target.checked)} />
          Required: a record cannot be saved without it
        </label>
        <div className="flex items-center justify-between pt-2">
          <button type="button" onClick={() => { onClose(); onAddLinked(); }} className="text-sm text-purple-700 dark:text-purple-300 hover:underline cursor-pointer">
            Link a field from another table instead
          </button>
          <div className="flex gap-2">
            <button type="button" onClick={onClose} className="px-3 py-2 rounded-lg border border-gray-300 dark:border-gray-700 text-sm cursor-pointer">
              Cancel
            </button>
            <button type="submit" className="px-4 py-2 rounded-lg bg-blue-600 hover:bg-blue-700 text-white text-sm font-semibold cursor-pointer">
              Add field
            </button>
          </div>
        </div>
      </form>
    </Drawer>
  );
};
