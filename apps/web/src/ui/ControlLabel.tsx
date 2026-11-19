export interface ControlLabelProps {
  id: string;
  /** The control's title, when the response carries it. */
  title?: string | null;
  /** Titles by control id, for a screen whose response does not carry them. */
  lookup?: Record<string, string>;
}

/** `AC-2 · Account Management`, as text. A control with no known title says so. */
export function ControlLabel({ id, title, lookup }: ControlLabelProps) {
  const name = title ?? lookup?.[id] ?? lookup?.[id.toLowerCase()];
  const shown = id.toUpperCase();
  return <span>{name ? `${shown} · ${name}` : `${shown} unknown`}</span>;
}
