export interface PersonLabelProps {
  id: string;
  lookup?: Record<string, string>;
}

export function PersonLabel({ id, lookup }: PersonLabelProps) {
  const name = lookup?.[id];
  return <span>{name ? `${name} <${id}>` : `${id} unknown`}</span>;
}
