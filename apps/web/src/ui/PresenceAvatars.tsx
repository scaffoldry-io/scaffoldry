import type { Present } from "../realtime";

/** How many faces the header shows before it counts the rest. */
const SHOWN = 5;

function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  const letters = parts.length > 1 ? parts[0][0] + parts[parts.length - 1][0] : (parts[0] ?? "?").slice(0, 2);
  return letters.toUpperCase();
}

/**
 * Who else has the app open (real-time phase 3): up to five avatars, each with the person's name in words, and `+3`
 * for the rest. It shows nothing when nobody is here.
 */
export function PresenceAvatars({ present }: { present: Present[] }) {
  if (present.length === 0) return null;
  const shown = present.slice(0, SHOWN);
  const more = present.length - shown.length;
  const label = present.length === 1 ? "1 person has this app open" : `${present.length} people have this app open`;
  return (
    <ul aria-label={label} className="flex items-center -space-x-1">
      {shown.map((p) => (
        <li key={p.address} title={p.name} className="flex items-center">
          <span aria-hidden="true" className="inline-flex h-7 w-7 items-center justify-center rounded-full border-2 border-white bg-blue-600 text-[11px] font-semibold text-white dark:border-slate-900">
            {initials(p.name)}
          </span>
          <span className="sr-only">{p.name}</span>
        </li>
      ))}
      {more > 0 && (
        <li aria-label={`and ${more} more`} className="ml-2 text-xs font-medium text-slate-600 dark:text-slate-300">
          <span aria-hidden="true">+{more}</span>
        </li>
      )}
    </ul>
  );
}
