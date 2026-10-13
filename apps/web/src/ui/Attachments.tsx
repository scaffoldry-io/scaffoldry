import { useCallback, useEffect, useId, useRef, useState, type DragEvent, type KeyboardEvent } from "react";
import { fetchRecordFile, listRecordFiles, uploadRecordFile, type RecordFile, type StoredFile } from "../api";
import { Banner } from "./Banner";
import { explainError } from "./explainError";

export type AttachedFile = StoredFile;

/** A size in words: `900 B`, `2 KB`, `1.5 MB`. */
export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const round = (v: number) => (Number.isInteger(v) ? String(v) : v.toFixed(1));
  if (n < 1024 * 1024) return `${round(n / 1024)} KB`;
  if (n < 1024 * 1024 * 1024) return `${round(n / 1024 / 1024)} MB`;
  return `${round(n / 1024 / 1024 / 1024)} GB`;
}

const SCAN_WORDS: Record<string, { icon: string; text: string }> = {
  pending: { icon: "⏳", text: "Scanning" },
  skipped: { icon: "⚠", text: "Not scanned" },
  error: { icon: "⚠", text: "Scan failed" },
  clean: { icon: "✓", text: "Scanned" },
};

/** The scan state as an icon and words. Never colour alone. */
function ScanMarker({ state }: { state: string | undefined }) {
  const word = state ? SCAN_WORDS[state] : undefined;
  if (!word) return null;
  return (
    <span className="inline-flex items-center gap-1 text-xs text-slate-600 dark:text-slate-400" data-testid={`scan-state-${state}`}>
      <span aria-hidden="true">{word.icon}</span>
      <span>{word.text}</span>
    </span>
  );
}

/** Files as chips: name and size, as text. For a grid cell. */
export function AttachmentChips({ value }: { value: AttachedFile[] }) {
  if (!value.length) return null;
  return (
    <ul className="flex flex-wrap gap-1" aria-label="Files">
      {value.map((f) => (
        <li key={f.ref_id} className="inline-flex items-center gap-1 rounded-full border border-slate-300 dark:border-slate-600 px-2 py-0.5 text-xs">
          <span aria-hidden="true">📎</span>
          <span className="truncate max-w-[12rem]">{f.name}</span>
          <span className="text-slate-500">{formatBytes(f.size)}</span>
        </li>
      ))}
    </ul>
  );
}

/** The first image of a record as a cover. It loads through the authorized route. Nothing shows when there is no image. */
export function AttachmentCover({ appSlug, recordId, value }: { appSlug: string; recordId: string; value: AttachedFile[] }) {
  const first = value.find((f) => f.media_type.startsWith("image/"));
  const [src, setSrc] = useState<string | null>(null);
  const refId = first?.ref_id;
  useEffect(() => {
    if (!refId) return;
    let current = true;
    let made: string | null = null;
    fetchRecordFile(appSlug, recordId, refId)
      .then((blob) => {
        if (!current) return;
        made = URL.createObjectURL(blob);
        setSrc(made);
      })
      .catch(() => current && setSrc(null));
    return () => {
      current = false;
      if (made) URL.revokeObjectURL(made);
    };
  }, [appSlug, recordId, refId]);
  if (!first || !src) return null;
  return <img src={src} alt={first.name} className="h-full w-full object-cover" />;
}

interface Upload {
  key: number;
  name: string;
  sent: number;
  total: number;
  controller: AbortController;
}

export interface AttachmentFieldProps {
  appSlug: string;
  recordId: string;
  fieldName: string;
  label: string;
  value: AttachedFile[];
  onChange: (next: AttachedFile[]) => void;
  readOnly?: boolean;
}

/**
 * The editor of an attachment field. A drop target and a `Choose files` button, a list with name, size, type and scan
 * state as text, `Remove`, and a download. A person who may not read the field sees nothing at all.
 */
export function AttachmentField({ appSlug, recordId, fieldName, label, value, onChange, readOnly }: AttachmentFieldProps) {
  const base = useId();
  const input = useRef<HTMLInputElement>(null);
  const latest = useRef(value);
  latest.current = value;
  const [readable, setReadable] = useState<boolean | null>(null);
  const [scan, setScan] = useState<Record<string, string>>({});
  const [uploads, setUploads] = useState<Upload[]>([]);
  const [problems, setProblems] = useState<{ name: string; message: string; next?: string }[]>([]);
  const [dragging, setDragging] = useState(false);
  const counter = useRef(0);

  useEffect(() => {
    let current = true;
    listRecordFiles(appSlug, recordId)
      .then((r) => {
        if (!current) return;
        setReadable(r.fields.includes(fieldName));
        setScan((prev) => ({ ...Object.fromEntries(r.files.map((f: RecordFile) => [f.ref_id, f.scan_state])), ...prev }));
      })
      .catch((e) => {
        if (!current) return;
        setReadable(true);
        setProblems((p) => [...p, { name: label, ...explainError(e) }]);
      });
    return () => {
      current = false;
    };
  }, [appSlug, recordId, fieldName, label]);

  const startUpload = useCallback(
    async (file: File) => {
      const controller = new AbortController();
      const key = ++counter.current;
      setUploads((u) => [...u, { key, name: file.name, sent: 0, total: file.size || 1, controller }]);
      try {
        const done = await uploadRecordFile(appSlug, file, {
          signal: controller.signal,
          onProgress: (sent, total) => setUploads((u) => u.map((x) => (x.key === key ? { ...x, sent, total } : x))),
        });
        setScan((s) => ({ ...s, [done.ref_id]: done.scan_state }));
        onChange([...latest.current, { ref_id: done.ref_id, name: done.name, size: done.size, media_type: done.media_type }]);
      } catch (e) {
        if (!(e instanceof DOMException && e.name === "AbortError")) {
          setProblems((p) => [...p, { name: file.name, ...explainError(e) }]);
        }
      } finally {
        setUploads((u) => u.filter((x) => x.key !== key));
      }
    },
    [appSlug, onChange],
  );

  const take = (files: FileList | File[] | null | undefined) => {
    if (!files) return;
    setProblems([]);
    Array.from(files).forEach((f) => void startUpload(f));
  };

  const download = async (f: AttachedFile) => {
    try {
      const blob = await fetchRecordFile(appSlug, recordId, f.ref_id);
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = f.name;
      a.click();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
    } catch (e) {
      setProblems((p) => [...p, { name: f.name, ...explainError(e) }]);
    }
  };

  // Nothing at all until the server has said the person may read the field, and nothing after it says they may not.
  if (readable !== true) return null;

  const open = () => input.current?.click();
  const onZoneKey = (e: KeyboardEvent) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      open();
    }
  };
  const onDrop = (e: DragEvent) => {
    e.preventDefault();
    setDragging(false);
    if (!readOnly) take(e.dataTransfer?.files);
  };

  return (
    <div
      className="p-3 rounded-xl border border-slate-200 dark:border-slate-800 bg-slate-50/50 dark:bg-slate-800/30 space-y-2 md:col-span-2"
      data-testid={`attachment-field-${fieldName}`}
    >
      <div className="text-xs font-semibold text-slate-700 dark:text-slate-300">{label}</div>
      {value.length > 0 && (
        <ul className="space-y-1" aria-label={label}>
          {value.map((f) => (
            <li key={f.ref_id} className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-lg border border-slate-200 dark:border-slate-700 px-2.5 py-1.5 text-xs">
              <span aria-hidden="true">📎</span>
              <button
                type="button"
                className="underline disabled:no-underline disabled:opacity-60"
                aria-label={`Download ${f.name}`}
                disabled={scan[f.ref_id] === "pending"}
                title={scan[f.ref_id] === "pending" ? "Available when the scan finishes." : undefined}
                onClick={() => void download(f)}
              >
                {f.name}
              </button>
              <span className="text-slate-500">{formatBytes(f.size)}</span>
              <span className="text-slate-500">{f.media_type}</span>
              <ScanMarker state={scan[f.ref_id]} />
              {!readOnly && (
                <button type="button" className="ml-auto text-slate-600 hover:text-red-600" aria-label={`Remove ${f.name}`} onClick={() => onChange(value.filter((x) => x.ref_id !== f.ref_id))}>
                  Remove
                </button>
              )}
            </li>
          ))}
        </ul>
      )}

      {uploads.map((u) => {
        const pct = Math.min(100, Math.round((u.sent / u.total) * 100));
        return (
          <div key={u.key} className="space-y-1 text-xs" data-testid="attachment-upload">
            <div className="flex items-center justify-between gap-2">
              <span>{`Uploading ${u.name}: ${pct}%`}</span>
              <button type="button" aria-label={`Cancel upload of ${u.name}`} onClick={() => u.controller.abort()}>
                Cancel
              </button>
            </div>
            <div role="progressbar" aria-label={`Uploading ${u.name}`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct} className="h-1.5 rounded bg-slate-200 dark:bg-slate-700">
              <div className="h-full rounded bg-blue-600" style={{ width: `${pct}%` }} />
            </div>
          </div>
        );
      })}

      {problems.map((p, i) => (
        <Banner key={i} kind="error">
          <strong>{p.name}</strong>: {p.message} {p.next}
        </Banner>
      ))}

      {!readOnly && (
        <div className="flex flex-wrap items-center gap-2">
          <div
            role="button"
            tabIndex={0}
            aria-label={`Drop files here or press Enter to choose files for ${label}`}
            aria-describedby={`${base}-hint`}
            onKeyDown={onZoneKey}
            onDragOver={(e) => {
              e.preventDefault();
              setDragging(true);
            }}
            onDragLeave={() => setDragging(false)}
            onDrop={onDrop}
            className={`flex-1 rounded-lg border-2 border-dashed px-3 py-2 text-xs ${dragging ? "border-blue-500 bg-blue-50 dark:bg-blue-950/30" : "border-slate-300 dark:border-slate-600"}`}
          >
            <span id={`${base}-hint`}>Drop files here</span>
          </div>
          <button type="button" className="rounded-lg border border-slate-300 dark:border-slate-600 px-3 py-1.5 text-xs font-medium" onClick={open}>
            Choose files
          </button>
          <input ref={input} type="file" multiple hidden tabIndex={-1} data-testid={`attachment-input-${fieldName}`} onChange={(e) => {
            take(e.target.files);
            e.target.value = "";
          }} />
        </div>
      )}
    </div>
  );
}
