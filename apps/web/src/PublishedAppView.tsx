import React, { useEffect, useState } from "react";
import type { RegisteredApp } from "./types";
import { MultiViewWorkspace } from "./MultiViewWorkspace";
import { PageFrame } from "./PageFrame";
import { Dashboard } from "./Dashboard";
import { FormView, type FieldMeta } from "./views/FormView";
import { BoardView } from "./views/BoardView";
import { CalendarView } from "./views/CalendarView";
import { GalleryView } from "./views/GalleryView";
import { apiClient, type AppView } from "./api";

/** A page of the app as the server stores it: a view of the app, or a custom page shown in a frame. */
interface NavPage {
  id: string;
  title: string;
  custom: boolean;
  /** The saved view a view page names. */
  viewId: string | null;
}

/** The pages in the server's shape. The older builder pages have no `kind`, so they are not listed here. */
function navPages(app: RegisteredApp): NavPage[] {
  const pages = (app.manifest.pages ?? []) as unknown as { id?: string; title?: string; kind?: Record<string, unknown> }[];
  return pages
    .filter((p) => p && typeof p.id === "string" && p.kind && typeof p.kind === "object")
    .map((p) => {
      const view = (p.kind as { View?: { view_id?: unknown } }).View?.view_id;
      return { id: p.id as string, title: p.title || (p.id as string), custom: "Custom" in (p.kind as object), viewId: typeof view === "string" ? view : null };
    });
}

/** The fields of a table, as a form or a board needs them. */
function tableOf(app: RegisteredApp, tableId: string | null) {
  type Raw = { name: string; label?: string; field_type?: unknown; required?: boolean; select_options?: { id: string; label: string }[] };
  const tables = (app.manifest.tables ?? []) as unknown as { id: string; slug?: string; fields: Raw[]; primary_field?: string | null }[];
  const table = tables.find((t) => t.id === tableId || t.slug === tableId) ?? tables[0];
  const fields: FieldMeta[] = (table?.fields ?? []).map((f) => ({
    name: f.name,
    label: f.label ?? f.name,
    field_type: typeof f.field_type === "string" ? f.field_type : Object.keys((f.field_type as object) ?? {})[0] ?? "Text",
    required: f.required,
    select_options: f.select_options?.map((o) => ({ id: o.id, label: o.label })),
  }));
  return { fields, title: table?.primary_field ?? fields[0]?.name ?? "" };
}

/** The saved view kinds a page can open. Any other view, and a view that cannot be found or loaded, shows the app's tables. */
const OPENS = ["dashboard", "form", "kanban", "calendar", "gallery"];

function ViewOpened({ app, view, onSaved }: { app: RegisteredApp; view: AppView; onSaved: (v: AppView) => void }) {
  const { fields, title } = tableOf(app, view.table_id);
  switch (view.kind) {
    case "form":
      return <FormView appSlug={app.slug} view={view} fields={fields} />;
    case "kanban": {
      const field = fields.find((f) => f.name === view.config?.kanban_column_field);
      return field ? <BoardView appSlug={app.slug} view={view} columnField={{ name: field.name, options: field.select_options ?? [] }} titleField={title} /> : <p role="alert" className="text-sm text-red-700">This board has no column field.</p>;
    }
    case "calendar":
      return <CalendarView appSlug={app.slug} view={view} titleField={title} />;
    case "gallery":
      return <GalleryView appSlug={app.slug} view={view} titleField={title} />;
    default:
      return <Dashboard appSlug={app.slug} view={view} canEdit onSaved={onSaved} />;
  }
}

/**
 * A page of kind View names a saved view. A dashboard, a form, a board, a calendar or a gallery is drawn as one. Any other
 * view, and a view that cannot be found or loaded, shows the app's tables as before.
 */
function ViewPage({ app, viewId, fallback }: { app: RegisteredApp; viewId: string; fallback: React.ReactNode }) {
  const [opened, setOpened] = useState<AppView | null>(null);
  useEffect(() => {
    let live = true;
    setOpened(null);
    apiClient
      .listViews(app.slug)
      .then((r) => live && setOpened(r.views.find((v) => v.id === viewId && OPENS.includes(v.kind)) ?? null))
      .catch(() => live && setOpened(null));
    return () => {
      live = false;
    };
  }, [app.slug, viewId]);
  // The server decides who may save a layout. A person who may not sees its refusal.
  if (opened) return <div className="px-6 py-4"><ViewOpened app={app} view={opened} onSaved={setOpened} /></div>;
  return <>{fallback}</>;
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
  const workspace = <MultiViewWorkspace app={app} onBack={() => onBackToDesk?.()} onOpenStudio={() => onOpenBuilder?.(app.slug)} />;
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
      ) : open?.viewId ? (
        <ViewPage key={open.id} app={app} viewId={open.viewId} fallback={workspace} />
      ) : (
        workspace
      )}
    </div>
  );
};
