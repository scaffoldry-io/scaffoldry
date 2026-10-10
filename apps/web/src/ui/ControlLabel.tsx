export interface ControlLabelProps {
  id: string;
  lookup?: Record<string, string>;
}

export function ControlLabel({ id, lookup }: ControlLabelProps) {
  const name = lookup?.[id];
  return <span>{name ? `${id} — ${name}` : `${id} unknown`}</span>;
}
