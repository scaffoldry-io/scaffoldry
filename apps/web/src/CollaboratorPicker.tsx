import { useEffect, useId, useState } from "react";
import { listCollaborators, type CollaboratorOption } from "./api";
import { ErrorState } from "./ui";

export interface CollaboratorValue {
  id: string;
  label: string;
  email?: string;
  active?: boolean;
}

export interface CollaboratorChipsProps {
  value: CollaboratorValue[];
  onRemove?: (id: string) => void;
}

/** Renders selected collaborators as chips with optional remove button. */
export function CollaboratorChips({ value, onRemove }: CollaboratorChipsProps) {
  return (
    <ul className="flex flex-wrap gap-1 mb-1" aria-label="Collaborators">
      {value.map((v) => {
        const isDeactivated = v.active === false;
        const displayText = isDeactivated
          ? `${v.label} (no longer active)`
          : v.email && v.email !== v.label
          ? `${v.label} <${v.email}>`
          : v.label;

        return (
          <li
            key={v.id}
            className={`inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-sm ${
              isDeactivated
                ? "bg-amber-50 text-amber-800 border-amber-300 dark:bg-amber-950 dark:text-amber-200"
                : "bg-slate-50 text-slate-700 border-slate-200 dark:bg-slate-800 dark:text-slate-200"
            }`}
          >
            <span>{displayText}</span>
            {onRemove && (
              <button
                type="button"
                aria-label={`Remove ${v.label}`}
                className="hover:text-red-500 font-bold ml-0.5"
                onClick={() => onRemove(v.id)}
              >
                ×
              </button>
            )}
          </li>
        );
      })}
    </ul>
  );
}

export interface CollaboratorPickerProps {
  appSlug: string;
  value: CollaboratorValue[];
  onChange: (next: CollaboratorValue[]) => void;
  cardinality?: "one" | "many";
  allowMultiple?: boolean;
}

/** Chooses collaborators by searching directory. Combobox with keyboard navigation. */
export function CollaboratorPicker({
  appSlug,
  value,
  onChange,
  cardinality = "many",
  allowMultiple,
}: CollaboratorPickerProps) {
  const isSingle = cardinality === "one" || allowMultiple === false;
  const base = useId();
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);
  const [options, setOptions] = useState<CollaboratorOption[] | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [active, setActive] = useState(0);

  useEffect(() => {
    if (!open) return;
    let current = true;
    listCollaborators(appSlug, query)
      .then((res) => {
        if (!current) return;
        setOptions(res.collaborators);
        setActive(0);
        setError(null);
      })
      .catch((e) => current && setError(e));
    return () => {
      current = false;
    };
  }, [appSlug, query, open]);

  const choose = (o: CollaboratorOption) => {
    const next: CollaboratorValue = {
      id: o.id,
      label: o.label,
      email: o.email,
      active: o.active ?? true,
    };
    if (isSingle) {
      onChange([next]);
    } else if (!value.some((v) => v.id === o.id)) {
      onChange([...value, next]);
    }
    setQuery("");
    setOpen(false);
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    const count = options?.length ?? 0;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setOpen(true);
      setActive((a) => (count ? (a + 1) % count : 0));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((a) => (count ? (a - 1 + count) % count : 0));
    } else if (e.key === "Enter" && open && options?.[active]) {
      e.preventDefault();
      choose(options[active]);
    } else if (e.key === "Escape") {
      setOpen(false);
    }
  };

  const status =
    !open || options === null
      ? ""
      : options.length === 0
      ? `No matches for "${query}"`
      : `${options.length} ${options.length === 1 ? "result" : "results"}`;
  const listId = `${base}-list`;

  return (
    <div className="relative collaborator-picker">
      <CollaboratorChips
        value={value}
        onRemove={(id) => onChange(value.filter((v) => v.id !== id))}
      />
      <input
        role="combobox"
        aria-label="Find collaborator"
        aria-expanded={open}
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={
          open && options?.[active] ? `${base}-${active}` : undefined
        }
        value={query}
        onChange={(e) => {
          setQuery(e.target.value);
          setOpen(true);
        }}
        onFocus={() => setOpen(true)}
        onKeyDown={onKeyDown}
        placeholder="Search directory..."
        className="px-2 py-1 text-sm border rounded w-full focus:outline-none focus:ring-1 focus:ring-blue-500"
      />
      <div role="status" aria-live="polite" className="text-xs text-gray-500 mt-0.5">
        {status}
      </div>
      {error !== null && <ErrorState error={error} />}
      {open && options && options.length > 0 && (
        <ul
          role="listbox"
          id={listId}
          className="absolute z-50 mt-1 max-h-60 w-full overflow-auto rounded border bg-white dark:bg-slate-900 shadow-lg text-sm"
        >
          {options.map((o, i) => (
            <li
              key={o.id}
              id={`${base}-${i}`}
              role="option"
              aria-selected={i === active}
              className={`px-3 py-1.5 cursor-pointer flex justify-between items-center ${
                i === active
                  ? "bg-blue-100 dark:bg-blue-900 text-blue-900 dark:text-blue-100"
                  : "hover:bg-gray-100 dark:hover:bg-slate-800"
              }`}
              onMouseDown={(e) => {
                e.preventDefault();
                choose(o);
              }}
            >
              <span className="font-medium">{o.label}</span>
              {o.email && (
                <span className="text-xs text-gray-500 ml-2">{o.email}</span>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
