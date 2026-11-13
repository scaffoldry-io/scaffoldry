import { useEffect, useState } from "react";
import { apiClient, type AppView, type ViewData } from "../api";

type Item = { ref_id?: string; name?: string; media_type?: string };

/** Up to two capital letters from a name, for a card with no picture. */
export function initialsOf(name: string): string {
  const letters = name.trim().split(/\s+/).filter(Boolean).slice(0, 2).map((w) => w[0].toUpperCase());
  return letters.join("") || "?";
}

/** The cover of a card: the first image of the attachment field, or the name's initials when there is none or it cannot load. */
function Cover({ appSlug, recordId, files, name }: { appSlug: string; recordId: string; files: Item[]; name: string }) {
  const first = files.find((f) => f.ref_id && (f.media_type ?? "").startsWith("image/"));
  const [src, setSrc] = useState<string | null>(null);
  useEffect(() => {
    if (!first?.ref_id) return;
    let live = true;
    let url: string | null = null;
    apiClient
      .fetchRecordFile(appSlug, recordId, first.ref_id)
      .then((blob) => {
        if (!live) return;
        url = URL.createObjectURL(blob);
        setSrc(url);
      })
      .catch(() => live && setSrc(null));
    return () => {
      live = false;
      if (url) URL.revokeObjectURL(url);
    };
  }, [appSlug, recordId, first?.ref_id]);
  if (first && src) return <img src={src} alt={first.name ?? name} className="h-32 w-full rounded-t object-cover" />;
  return (
    <div aria-hidden="true" className="flex h-32 w-full items-center justify-center rounded-t bg-slate-200 text-2xl font-semibold text-slate-700 dark:bg-slate-800 dark:text-slate-200">
      {initialsOf(name)}
    </div>
  );
}

/** A gallery: a card for each row, with a cover from the first image of an attachment field and the fields the view chose. */
export function GalleryView({ appSlug, view, titleField }: { appSlug: string; view: AppView; titleField: string }) {
  const [data, setData] = useState<ViewData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const coverField = String(view.config?.gallery_cover_field ?? "");
  const cardFields: string[] = Array.isArray(view.config?.gallery_card_fields) ? (view.config.gallery_card_fields as string[]) : [];

  useEffect(() => {
    let live = true;
    apiClient
      .runView(appSlug, view.id, { limit: 60 })
      .then((d) => live && setData(d))
      .catch((e: unknown) => live && setError(e instanceof Error ? e.message : "The gallery could not be loaded."));
    return () => {
      live = false;
    };
  }, [appSlug, view.id]);

  if (error) return <p role="alert" className="text-sm text-red-700">{error}</p>;
  if (!data) return <p className="text-sm text-slate-500">Loading…</p>;
  return (
    <ul className="grid grid-cols-2 gap-3 md:grid-cols-4">
      {(data.records ?? []).map((r) => {
        const title = String(r.data[titleField] ?? "");
        const files = Array.isArray(r.data[coverField]) ? (r.data[coverField] as Item[]) : [];
        return (
          <li key={r.id} className="rounded border">
            <Cover appSlug={appSlug} recordId={r.id} files={files} name={title} />
            <div className="p-2 text-sm">
              <div className="font-medium">{title}</div>
              {cardFields.map((f) => (r.data[f] === undefined || r.data[f] === null ? null : <div key={f} className="text-xs text-slate-600">{String(r.data[f])}</div>))}
            </div>
          </li>
        );
      })}
    </ul>
  );
}
