import React, { useState } from "react";
import type { RegisteredApp } from "./types";
import { MultiViewWorkspace } from "./MultiViewWorkspace";
import { PageFrame } from "./PageFrame";

/** A page of the app as the server stores it: a view of the app, or a custom page shown in a frame. */
interface NavPage {
  id: string;
  title: string;
  custom: boolean;
}

/** The pages in the server's shape. The older builder pages have no `kind`, so they are not listed here. */
function navPages(app: RegisteredApp): NavPage[] {
  const pages = (app.manifest.pages ?? []) as unknown as { id?: string; title?: string; kind?: Record<string, unknown> }[];
  return pages
    .filter((p) => p && typeof p.id === "string" && p.kind && typeof p.kind === "object")
    .map((p) => ({ id: p.id as string, title: p.title || (p.id as string), custom: "Custom" in (p.kind as object) }));
}

interface PublishedAppViewProps {
  app: RegisteredApp;
  onOpenBuilder?: (slug: string) => void;
  onOpenIntakeForm?: (tableId?: string) => void;
  onBackToDesk?: () => void;
}

/**
 * The published app screen: the app's own tables, and the rows the server holds for it. It builds its columns
 * from the app's definition, so it works for an app of any shape. It holds no rows of its own.
 */
export const PublishedAppView: React.FC<PublishedAppViewProps> = ({ app, onOpenBuilder, onOpenIntakeForm, onBackToDesk }) => {
  const pages = navPages(app);
  const [pageId, setPageId] = useState<string | null>(null);
  const open = pages.find((p) => p.id === (pageId ?? pages[0]?.id));
  return (
    <div data-testid="published-app-runtime" className="min-h-screen bg-slate-50 dark:bg-slate-950">
      {onOpenIntakeForm && (
        <div className="px-6 pt-3 flex justify-end">
          <button
            type="button"
            data-testid="open-public-intake-form-btn"
            onClick={() => onOpenIntakeForm()}
            className="px-3 py-1.5 rounded-lg border border-blue-200 bg-blue-50 text-blue-700 text-xs font-semibold hover:bg-blue-100 cursor-pointer"
          >
            📋 Public Intake Form
          </button>
        </div>
      )}
      {pages.length > 0 && (
        <div role="tablist" aria-label="Pages" className="px-6 pt-3 flex flex-wrap gap-2">
          {pages.map((p) => (
            <button
              key={p.id}
              type="button"
              role="tab"
              aria-selected={open?.id === p.id}
              onClick={() => setPageId(p.id)}
              className={`px-3 py-1.5 rounded-lg border text-xs font-semibold cursor-pointer ${
                open?.id === p.id ? "border-blue-300 bg-blue-50 text-blue-700" : "border-slate-200 bg-white text-slate-600 hover:bg-slate-50"
              }`}
            >
              {p.title}
            </button>
          ))}
        </div>
      )}
      {open?.custom ? (
        <div className="px-6 py-4">
          <PageFrame key={open.id} appSlug={app.slug} pageId={open.id} />
        </div>
      ) : (
        <MultiViewWorkspace app={app} onBack={() => onBackToDesk?.()} onOpenStudio={() => onOpenBuilder?.(app.slug)} />
      )}
    </div>
  );
};
