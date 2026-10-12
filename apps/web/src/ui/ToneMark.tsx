import { StatusBadge, BadgeTone } from "./StatusBadge";

/** A rule that matched a row, as the server reports it in `_tone`. */
export interface AppliedTone {
  id: string;
  label: string;
  tone: "neutral" | "info" | "success" | "warning" | "danger";
  target: "row" | "field";
  field?: string;
}

const BADGE: Record<AppliedTone["tone"], BadgeTone> = {
  neutral: "neutral",
  info: "info",
  success: "ok",
  warning: "warn",
  danger: "bad",
};

/** The rule labels that apply to a row or to one of its fields, for an accessible name. */
export function toneLabels(tones: AppliedTone[] | undefined, field?: string): string[] {
  return (tones ?? []).filter((t) => (field === undefined ? t.target === "row" : t.target === "field" && t.field === field)).map((t) => t.label);
}

/** A row's name for assistive technology: its own name, then every rule that marks it. */
export function nameWithTones(name: string, tones: AppliedTone[] | undefined): string {
  const labels = (tones ?? []).map((t) => t.label);
  return labels.length ? `${name}. ${labels.join(", ")}` : name;
}

/**
 * The rules that mark a row or a field. Color is never the only signal: each rule shows an icon and its label as
 * text, and the group is named for the same labels.
 */
export function ToneMark({ tones, field }: { tones?: AppliedTone[]; field?: string }) {
  const shown = (tones ?? []).filter((t) => (field === undefined ? t.target === "row" : t.target === "field" && t.field === field));
  if (shown.length === 0) return null;
  return (
    <span role="group" aria-label={`Marked by: ${shown.map((t) => t.label).join(", ")}`} className="inline-flex flex-wrap gap-1">
      {shown.map((t) => (
        <StatusBadge key={t.id} tone={BADGE[t.tone]} text={t.label} />
      ))}
    </span>
  );
}
