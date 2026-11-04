import { useEffect, useState } from "react";
import { apiClient, type RecordButton } from "./api";

/**
 * A record's button fields (field types phase 5). Each shows its label. One the person may not press is disabled, with
 * the reason in words. Pressing starts the rule the button names; the server checks the person and the record again.
 */
export function RecordButtons({ appSlug, recordId, onPressed }: { appSlug: string; recordId: string; onPressed?: () => void }) {
  const [buttons, setButtons] = useState<RecordButton[]>([]);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    apiClient
      .listRecordButtons(appSlug, recordId)
      .then((b) => live && setButtons(b))
      .catch(() => live && setButtons([]));
    return () => {
      live = false;
    };
  }, [appSlug, recordId]);

  if (buttons.length === 0) return null;

  const press = async (b: RecordButton) => {
    setBusy(b.field);
    setMessage(null);
    try {
      const res = await apiClient.pressRecordButton(appSlug, recordId, b.field);
      setMessage(res.started ? "Started." : "It is already waiting for a decision.");
      onPressed?.();
    } catch (e) {
      setMessage(e instanceof Error ? e.message : "The button could not start its process.");
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="space-y-2" data-testid="record-buttons">
      <div className="flex flex-wrap gap-2">
        {buttons.map((b) => (
          <div key={b.field} className="flex flex-col gap-1">
            <button
              type="button"
              disabled={!b.enabled || busy !== null}
              title={b.reason ?? undefined}
              onClick={() => press(b)}
              className="rounded bg-blue-600 px-3 py-1.5 text-xs font-semibold text-white disabled:cursor-not-allowed disabled:bg-slate-300 disabled:text-slate-600 dark:disabled:bg-slate-700 dark:disabled:text-slate-300"
            >
              {b.label}
            </button>
            {!b.enabled && b.reason && <span className="max-w-xs text-[11px] text-slate-500">{b.reason}</span>}
          </div>
        ))}
      </div>
      {message && (
        <p role="status" className="text-xs text-slate-600 dark:text-slate-300">
          {message}
        </p>
      )}
    </div>
  );
}

/** A button field in a grid cell. It presses straight away; when the server refuses, the reason shows in place. */
export function GridButton({ appSlug, recordId, field, label }: { appSlug: string; recordId: string; field: string; label: string }) {
  const [note, setNote] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  return (
    <span className="inline-flex items-center gap-2">
      <button
        type="button"
        disabled={busy}
        onClick={async (e) => {
          e.stopPropagation();
          setBusy(true);
          try {
            const res = await apiClient.pressRecordButton(appSlug, recordId, field);
            setNote(res.started ? "Started." : "Already waiting.");
          } catch (err) {
            setNote(err instanceof Error ? err.message : "Not started.");
          } finally {
            setBusy(false);
          }
        }}
        className="rounded bg-blue-600 px-2 py-0.5 text-[11px] font-semibold text-white disabled:bg-slate-300"
      >
        {label}
      </button>
      {note && <span className="truncate text-[11px] text-slate-500" title={note}>{note}</span>}
    </span>
  );
}
