export interface UnitLabelProps {
  id: string;
  lookup?: Record<string, string>;
}

export function UnitLabel({ id, lookup }: UnitLabelProps) {
  const name = lookup?.[id];
  return <span>{name ? `${name} (${id})` : `${id} unknown`}</span>;
}
