import { useCallback, useEffect, useState } from "react";
import { apiClient, type HoldScope, type LegalHold } from "../api";
import { Banner, ConfirmAction, EmptyState, ErrorState, PageHeader, Skeleton, StatusBadge } from "../ui";

const buttonClass = "px-3 py-1.5 text-sm font-medium rounded border border-gray-300 dark:border-gray-700 disabled:opacity-50";
const inputClass = "w-full text-sm rounded border border-gray-300 dark:border-gray-700 p-2 bg-white dark:bg-gray-800";

function records(n: number): string {
  return `${n.toLocaleString("en-US")} ${n === 1 ? "record" : "records"}`;
}

type Draft = { name: string; case_ref: string; reason: string; scope_kind: HoldScope["scope_kind"]; workspace_id: string; app_slug: string; table_id: string; field: string; value: string; include_future: boolean };

const empty: Draft = { name: "", case_ref: "", reason: "", scope_kind: "app", workspace_id: "", app_slug: "", table_id: "", field: "", value: "", include_future: false };

function scopeOf(d: Draft): HoldScope {
  const s: HoldScope = { scope_kind: d.scope_kind, include_future: d.include_future };
  if (d.scope_kind === "workspace") s.workspace_id = d.workspace_id.trim();
  else s.app_slug = d.app_slug.trim();
  if ((d.scope_kind === "table" || d.scope_kind === "query") && d.table_id.trim()) s.table_id = d.table_id.trim();
  if (d.scope_kind === "query") s.filter = { [d.field.trim()]: d.value };
  return s;
}

/** Waits for a preview job and returns its sentence. */
async function previewSentence(scope: HoldScope): Promise<string> {
  const { job_id } = await apiClient.previewHold(scope);
  for (let i = 0; i < 120; i++) {
    const job = await apiClient.getAdminJob(job_id);
    if (job.state === "done") return String(job.result?.sentence ?? "");
    if (job.state === "failed") throw new Error("The preview could not be counted.");
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error("The preview is taking too long. Try again.");
}

// The Legal holds section of the console. A held record may go to trash and come back. It is never destroyed.
export function LegalHolds() {
  const [rows, setRows] = useState<LegalHold[] | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [sentence, setSentence] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [releasing, setReleasing] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setRows(await apiClient.listHolds());
      setError(null);
    } catch (e) {
      setError(e);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const set = (patch: Partial<Draft>) => {
    setDraft((d) => (d ? { ...d, ...patch } : d));
    setSentence(null);
  };

  const preview = async () => {
    if (!draft) return;
    setBusy(true);
    try {
      setSentence(await previewSentence(scopeOf(draft)));
      setError(null);
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  };

  const save = async () => {
    if (!draft) return;
    setBusy(true);
    try {
      await apiClient.placeHold({ ...scopeOf(draft), name: draft.name.trim(), case_ref: draft.case_ref.trim(), reason: draft.reason.trim() });
      setDraft(null);
      setSentence(null);
      await load();
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="space-y-4">
      <PageHeader title="Legal holds" description="A held record may go to trash and come back. Retention never destroys it while the hold stands." />
      {error !== null && <ErrorState error={error} />}
      {!draft && (
        <button type="button" className={buttonClass} onClick={() => setDraft({ ...empty })}>
          New hold
        </button>
      )}
      {draft && (
        <form
          className="space-y-3 rounded-lg border p-4"
          onSubmit={(e) => {
            e.preventDefault();
            void save();
          }}
        >
          <label className="block text-xs font-medium">
            Name
            <input className={inputClass} value={draft.name} onChange={(e) => set({ name: e.target.value })} />
          </label>
          <label className="block text-xs font-medium">
            Case reference
            <input className={inputClass} value={draft.case_ref} onChange={(e) => set({ case_ref: e.target.value })} />
          </label>
          <label className="block text-xs font-medium">
            Covers
            <select className={inputClass} value={draft.scope_kind} onChange={(e) => set({ scope_kind: e.target.value as Draft["scope_kind"] })}>
              <option value="app">An app</option>
              <option value="workspace">A workspace</option>
              <option value="table">A table</option>
              <option value="query">Records that match</option>
            </select>
          </label>
          {draft.scope_kind === "workspace" ? (
            <label className="block text-xs font-medium">
              Workspace
              <input className={inputClass} value={draft.workspace_id} onChange={(e) => set({ workspace_id: e.target.value })} />
            </label>
          ) : (
            <label className="block text-xs font-medium">
              App
              <input className={inputClass} value={draft.app_slug} onChange={(e) => set({ app_slug: e.target.value })} />
            </label>
          )}
          {(draft.scope_kind === "table" || draft.scope_kind === "query") && (
            <label className="block text-xs font-medium">
              Table
              <input className={inputClass} value={draft.table_id} onChange={(e) => set({ table_id: e.target.value })} />
            </label>
          )}
          {draft.scope_kind === "query" && (
            <div className="grid grid-cols-2 gap-2">
              <label className="block text-xs font-medium">
                Field
                <input className={inputClass} value={draft.field} onChange={(e) => set({ field: e.target.value })} />
              </label>
              <label className="block text-xs font-medium">
                Value
                <input className={inputClass} value={draft.value} onChange={(e) => set({ value: e.target.value })} />
              </label>
            </div>
          )}
          {(draft.scope_kind === "table" || draft.scope_kind === "query") && (
            <label className="flex items-center gap-2 text-xs">
              <input type="checkbox" checked={draft.include_future} onChange={(e) => set({ include_future: e.target.checked })} />
              Also hold records written later
            </label>
          )}
          <label className="block text-xs font-medium">
            Reason
            <textarea className={inputClass} rows={2} value={draft.reason} onChange={(e) => set({ reason: e.target.value })} />
          </label>
          {sentence && <Banner kind="info">{sentence}</Banner>}
          <div className="flex gap-2">
            <button type="button" className={buttonClass} disabled={busy} onClick={() => void preview()}>
              Preview
            </button>
            <button type="submit" className={buttonClass} disabled={busy || !sentence || !draft.name.trim() || !draft.reason.trim()}>
              Place hold
            </button>
            <button type="button" className={buttonClass} onClick={() => setDraft(null)}>
              Cancel
            </button>
          </div>
        </form>
      )}
      {rows === null ? (
        <Skeleton rows={3} />
      ) : rows.length === 0 ? (
        <EmptyState title="No legal holds" description="Retention runs on its policies. Place a hold when counsel asks to keep records." />
      ) : (
        <ul className="space-y-2">
          {rows.map((h) => (
            <li key={h.id} className="rounded-lg border p-3 space-y-1">
              <div className="flex items-center justify-between gap-2">
                <span className="font-semibold">{h.name}</span>
                <StatusBadge tone={h.active ? "warn" : "neutral"} text={h.active ? "Held" : "Released"} />
              </div>
              <div className="text-sm">{h.scope}</div>
              <div className="text-xs text-gray-600 dark:text-gray-400">
                <span>{records(h.count)}</span>
                {h.case_ref && ` · ${h.case_ref}`} · placed by {h.created_by} on {h.created_at.slice(0, 10)}
                {h.released_by && ` · released by ${h.released_by}`}
              </div>
              {h.active &&
                (releasing === h.id ? (
                  <ConfirmAction
                    verb="Release"
                    target={h.name}
                    consequence="Retention may destroy these records again on its next run. A hold is released by a second person: someone other than the person who placed it."
                    audited
                    requireReason
                    onConfirm={async (reason) => {
                      await apiClient.releaseHold(h.id, reason);
                    }}
                    onSuccess={() => {
                      setReleasing(null);
                      void load();
                    }}
                  />
                ) : (
                  <button type="button" className={buttonClass} onClick={() => setReleasing(h.id)}>
                    Release
                  </button>
                ))}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
