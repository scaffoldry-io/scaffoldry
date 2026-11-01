import { useEffect, useId, useState } from "react";
import { listLinkOptions, type LinkOption } from "./api";
import { ErrorState } from "./ui";

export interface LinkChipsProps {
  value: { id: string; label: string }[];
  /** True when the person can read the target record. Without it the chip is a label only. */
  canOpen?: (id: string) => boolean;
  onOpen?: (id: string) => void;
  onRemove?: (id: string) => void;
}

/** A link as chips. A chip for a record the person cannot open shows its label and no link. */
export function LinkChips({ value, canOpen, onOpen, onRemove }: LinkChipsProps) {
  return (
    <ul className="flex flex-wrap gap-1" aria-label="Linked records">
      {value.map((v) => (
        <li key={v.id} className="inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-sm">
          {canOpen?.(v.id) && onOpen ? (
            <button type="button" className="underline" onClick={() => onOpen(v.id)}>
              {v.label}
            </button>
          ) : (
            <span>{v.label}</span>
          )}
          {onRemove && (
            <button type="button" aria-label={`Remove ${v.label}`} onClick={() => onRemove(v.id)}>
              ×
            </button>
          )}
        </li>
      ))}
    </ul>
  );
}

export interface LinkPickerProps {
  appSlug: string;
  /** The table the link points at. */
  tableId: string;
  value: { id: string; label: string }[];
  onChange: (next: { id: string; label: string }[]) => void;
  /** `one` replaces the chip. `many` adds to them. */
  cardinality?: "one" | "many";
  canOpen?: (id: string) => boolean;
  onOpen?: (id: string) => void;
  /** Puts the cursor in the search box when the picker appears, as a grid cell's editor does. */
  autoFocus?: boolean;
  /** Called on Escape, after the list closes, so an editor can close too and hand focus back. */
  onEscape?: () => void;
}

/** Chooses the record a link points at. A combobox: type to search, arrows to move, Enter to choose, Escape to close. */
export function LinkPicker({ appSlug, tableId, value, onChange, cardinality = "many", canOpen, onOpen, autoFocus, onEscape }: LinkPickerProps) {
  const base = useId();
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);
  const [options, setOptions] = useState<LinkOption[] | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [active, setActive] = useState(0);

  useEffect(() => {
    if (!open) return;
    let current = true;
    listLinkOptions(appSlug, tableId, query)
      .then((page) => {
        if (!current) return;
        setOptions(page.options);
        setActive(0);
        setError(null);
      })
      .catch((e) => current && setError(e));
    return () => {
      current = false;
    };
  }, [appSlug, tableId, query, open]);

  const choose = (o: LinkOption) => {
    const next = { id: o.id, label: o.label };
    if (cardinality === "one") onChange([next]);
    else if (!value.some((v) => v.id === o.id)) onChange([...value, next]);
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
      onEscape?.();
    }
  };

  const status = !open || options === null ? "" : options.length === 0 ? `No matches for "${query}"` : `${options.length} ${options.length === 1 ? "result" : "results"}`;
  const listId = `${base}-list`;

  return (
    <div>
      <LinkChips value={value} canOpen={canOpen} onOpen={onOpen} onRemove={(id) => onChange(value.filter((v) => v.id !== id))} />
      <input
        role="combobox"
        aria-label="Find a record to link"
        aria-expanded={open}
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={open && options?.[active] ? `${base}-${active}` : undefined}
        value={query}
        onChange={(e) => {
          setQuery(e.target.value);
          setOpen(true);
        }}
        onFocus={() => setOpen(true)}
        onKeyDown={onKeyDown}
        autoFocus={autoFocus}
      />
      <div role="status" aria-live="polite" className="text-sm">
        {status}
      </div>
      {error !== null && <ErrorState error={error} />}
      {open && options && options.length > 0 && (
        <ul role="listbox" id={listId}>
          {options.map((o, i) => (
            <li
              key={o.id}
              id={`${base}-${i}`}
              role="option"
              aria-selected={i === active}
              onMouseDown={(e) => {
                e.preventDefault();
                choose(o);
              }}
            >
              {o.label}
              {o.secondary && <span className="text-gray-500"> {o.secondary}</span>}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
