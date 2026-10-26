import { useEffect, useState } from "react";
import { apiClient, type LibraryRuleReview } from "../api";
import { Banner, Drawer, ErrorState, FormField, Skeleton } from "../ui";

export interface ReviewDrawerProps {
  /** The rule to review. The drawer is closed when this is null. */
  ruleId: string | null;
  onClose: () => void;
  onDecided?: () => void;
}

const buttonClass = "px-3 py-1.5 text-sm font-medium rounded border disabled:opacity-50 disabled:cursor-not-allowed";
const inputClass = "w-full rounded border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-900 p-2 text-sm";

/** A reviewer reads the statement and the passages it cites in context, then publishes or rejects it. */
export function ReviewDrawer({ ruleId, onClose, onDecided }: ReviewDrawerProps) {
  const [view, setView] = useState<LibraryRuleReview | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [statement, setStatement] = useState("");
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [reworded, setReworded] = useState<number | null>(null);

  useEffect(() => {
    setView(null);
    setError(null);
    setNote("");
    setReworded(null);
    if (ruleId === null) return;
    let live = true;
    apiClient
      .libraryRuleReview(ruleId)
      .then((v) => {
        if (!live) return;
        setView(v);
        setStatement(v.rule.statement);
      })
      .catch((e) => live && setError(e));
    return () => {
      live = false;
    };
  }, [ruleId]);

  const decide = async (decision: "publish" | "reject") => {
    if (!view || ruleId === null) return;
    setBusy(true);
    try {
      const body: { decision: "publish" | "reject"; note: string; statement?: string } = { decision, note: note.trim() };
      if (decision === "publish" && statement.trim() !== view.rule.statement) body.statement = statement.trim();
      const done = await apiClient.libraryDecide(ruleId, body);
      setError(null);
      if (done.reworded) setReworded(done.rule.version);
      onDecided?.();
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  };

  const supplied = view?.citations.some((c) => c.text_origin === "supplied") ?? false;
  const fromAgent = view?.submitted_via === "agent";
  const noteMissing = note.trim() === "";

  return (
    <Drawer open={ruleId !== null} onClose={onClose} title="Review a rule">
      <div className="space-y-4" data-testid="review-drawer">
        {error !== null && <ErrorState error={error} />}
        {view === null && error === null && <Skeleton />}
        {view && (
          <>
            {fromAgent && <Banner kind="info">Proposed by an AI tool used by {view.submitted_by_name}.</Banner>}
            {supplied && <Banner kind="warning">This text was typed in by a person. Check it against the original.</Banner>}
            {reworded !== null && (
              <Banner kind="info">Your wording is saved as version {reworded}. A different reviewer must publish it.</Banner>
            )}
            <div className="grid gap-4 lg:grid-cols-2">
              <section aria-label="Statement" className="space-y-3">
                <div className="text-xs text-gray-500">
                  {view.policy.title}, rule {view.rule.key}, version {view.rule.version}. Submitted by {view.submitted_by_name}.
                </div>
                <FormField id="library-statement" label="Statement" hint="You may reword it. A reworded rule goes to a different reviewer.">
                  {(props) => <textarea {...props} rows={4} className={inputClass} value={statement} onChange={(e) => setStatement(e.target.value)} />}
                </FormField>
                {view.previous && (
                  <div className="text-sm">
                    <div className="text-xs font-medium text-gray-500">Published now (version {view.previous.version})</div>
                    <p className="text-gray-900 dark:text-gray-100">{view.previous.statement}</p>
                  </div>
                )}
                {view.questions.length > 0 && (
                  <div className="space-y-2 text-sm">
                    <div className="text-xs font-medium text-gray-500">Questions about this policy</div>
                    {view.questions.map((q) => (
                      <div key={q.id}>
                        <div className="text-gray-900 dark:text-gray-100">{q.question}</div>
                        <div className="text-xs text-gray-500">{q.answer === null ? "Not answered yet" : q.answer}</div>
                      </div>
                    ))}
                  </div>
                )}
              </section>
              <section aria-label="Cited passages" className="space-y-3">
                {view.citations.map((c, i) => (
                  <figure key={`${c.snapshot_id}-${i}`} data-testid={`citation-${i}`} className="border border-gray-200 dark:border-gray-800 rounded-lg p-3 text-sm">
                    <blockquote className="whitespace-pre-wrap text-gray-700 dark:text-gray-300">
                      {c.before}
                      <mark className="bg-yellow-200 dark:bg-yellow-700 text-inherit">{c.passage}</mark>
                      {c.after}
                    </blockquote>
                    <figcaption className="mt-2 text-xs text-gray-500">
                      {c.source_title ?? "Source"}
                      {c.locator ? `, ${c.locator}` : ""}
                    </figcaption>
                  </figure>
                ))}
              </section>
            </div>
            <FormField id="library-note" label="Note" required hint="Why you are publishing or rejecting it. It goes in the record.">
              {(props) => <textarea {...props} rows={2} className={inputClass} value={note} onChange={(e) => setNote(e.target.value)} />}
            </FormField>
            <div className="flex gap-2">
              <button type="button" className={`${buttonClass} bg-blue-600 text-white border-blue-600`} disabled={busy || noteMissing} onClick={() => void decide("publish")}>
                Publish
              </button>
              <button type="button" className={`${buttonClass} border-gray-300 dark:border-gray-700`} disabled={busy || noteMissing} onClick={() => void decide("reject")}>
                Reject
              </button>
            </div>
          </>
        )}
      </div>
    </Drawer>
  );
}
