// A grant in a sentence a person can read. One function, so the form, the list and the explanation say the same thing.

export interface SentenceParts {
  /** Who holds it: a name, an address, or "Every department chair". */
  holder: string;
  role: string;
  /** Where it is anchored, in words: a unit's name, "the Biology workspace", "their own unit". */
  anchor: string;
  /** `unit`, `container` or `holder_unit`. */
  anchorKind: string;
  depth: number | null;
  domain: string | null;
  clearance: string | null;
}

const VERBS: Record<string, string> = {
  viewer: "view",
  commenter: "comment on",
  editor: "edit",
  admin: "administer",
  owner: "own",
};

function reach(depth: number | null): string {
  if (depth === null) return " and the units below it";
  if (depth === 0) return " itself";
  if (depth === 1) return " and the units one level below it";
  return ` and the units up to ${depth} levels below it`;
}

export function grantSentence(g: SentenceParts): string {
  const verb = VERBS[g.role] ?? g.role;
  const where =
    g.anchorKind === "container" ? ` in ${g.anchor}` : ` placed in ${g.anchor}${reach(g.depth)}`;
  const limits = [g.domain ? `in ${g.domain}` : null, g.clearance ? `up to ${g.clearance}` : null].filter(Boolean);
  return `${g.holder} can ${verb} everything${where}${limits.length ? `, ${limits.join(", ")}` : ""}.`;
}
