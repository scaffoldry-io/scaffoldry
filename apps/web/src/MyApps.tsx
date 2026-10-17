import React from "react";
import type { MyApp } from "./api";

interface MyAppsProps {
  apps: MyApp[];
  pinned: string[];
  onTogglePin: (slug: string) => void;
  onOpen: (slug: string) => void;
}

const HOW: Record<MyApp["scope"], string> = {
  admin: "Platform administrator access",
  workspace: "Through a workspace role",
  app: "Granted on this app only",
};

function Row({ app, isPinned, onTogglePin, onOpen }: { app: MyApp; isPinned: boolean; onTogglePin: (s: string) => void; onOpen: (s: string) => void }) {
  return (
    <li className="flex items-start justify-between gap-4 rounded-xl border border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-900 px-4 py-3">
      <div className="min-w-0">
        <div className="text-sm font-semibold text-slate-900 dark:text-slate-100 truncate">{app.title}</div>
        {app.description && <div className="text-xs text-slate-500 mt-0.5 line-clamp-2">{app.description}</div>}
        <div className="text-[11px] text-slate-500 mt-1">
          {app.department && <span className="mr-2">{app.department}</span>}
          <span className="px-1.5 py-0.5 rounded bg-slate-100 dark:bg-slate-800">{HOW[app.scope]}</span>
        </div>
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <button
          type="button"
          aria-label={`${isPinned ? "Unpin" : "Pin"} ${app.title}`}
          aria-pressed={isPinned}
          onClick={() => onTogglePin(app.slug)}
          className="px-2 py-1 rounded-lg text-xs border border-slate-300 dark:border-slate-700 hover:bg-slate-100 dark:hover:bg-slate-800 cursor-pointer"
        >
          {isPinned ? "📌 Pinned" : "📍 Pin"}
        </button>
        <button
          type="button"
          aria-label={`Open ${app.title}`}
          onClick={() => onOpen(app.slug)}
          className="px-3 py-1 rounded-lg text-xs font-semibold bg-blue-600 text-white hover:bg-blue-700 cursor-pointer"
        >
          Open
        </button>
      </div>
    </li>
  );
}

/** Every app the person can reach, pinned ones first. An app granted on its own appears here and nowhere else. */
export const MyApps: React.FC<MyAppsProps> = ({ apps, pinned, onTogglePin, onOpen }) => {
  const pinnedApps = apps.filter((a) => pinned.includes(a.slug));
  const others = apps.filter((a) => !pinned.includes(a.slug));
  return (
    <div data-testid="my-apps-view" className="p-6 max-w-4xl mx-auto space-y-6">
      <div>
        <h1 className="text-lg font-bold text-slate-900 dark:text-slate-100">My apps</h1>
        <p className="text-xs text-slate-500">The apps you can open, and how you can reach each one.</p>
      </div>
      {apps.length === 0 && <p className="text-sm text-slate-500">No apps yet. When someone gives you access to an app, it appears here.</p>}
      {pinnedApps.length > 0 && (
        <section data-testid="my-apps-pinned" className="space-y-2">
          <h2 className="text-[11px] font-semibold uppercase tracking-wider text-slate-400">Pinned</h2>
          <ul className="space-y-2">
            {pinnedApps.map((a) => (
              <Row key={a.slug} app={a} isPinned onTogglePin={onTogglePin} onOpen={onOpen} />
            ))}
          </ul>
        </section>
      )}
      {others.length > 0 && (
        <section className="space-y-2">
          <h2 className="text-[11px] font-semibold uppercase tracking-wider text-slate-400">{pinnedApps.length > 0 ? "All apps" : "Apps"}</h2>
          <ul className="space-y-2">
            {others.map((a) => (
              <Row key={a.slug} app={a} isPinned={false} onTogglePin={onTogglePin} onOpen={onOpen} />
            ))}
          </ul>
        </section>
      )}
    </div>
  );
};
