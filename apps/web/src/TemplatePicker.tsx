import { useEffect, useState } from "react";
import { apiClient, type AppTemplate, type TemplateUse } from "./api";
import { EmptyState, ErrorState, Skeleton } from "./ui";

/** Approved templates by category: what each holds in sentences, and a button to use it in this workspace. */
export function TemplatePicker({ workspaceId, onClose }: { workspaceId: string; onClose: () => void }) {
  const [templates, setTemplates] = useState<AppTemplate[] | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{ name: string; use: TemplateUse } | null>(null);
  const [failure, setFailure] = useState<string | null>(null);

  useEffect(() => {
    apiClient.templates().then((r) => setTemplates(r.templates), setError);
  }, []);

  const use = async (t: AppTemplate) => {
    setBusy(true);
    setFailure(null);
    try {
      setResult({ name: t.name, use: await apiClient.useTemplate(workspaceId, t.id, { approve: true }) });
    } catch (e) {
      setFailure(e instanceof Error ? e.message : "The template could not be used.");
    } finally {
      setBusy(false);
    }
  };

  const raised = result?.use.checks.filter((c) => c.outcome === "fail") ?? [];
  const categories = [...new Set((templates ?? []).map((t) => t.category))];

  return (
    <div role="dialog" aria-label="Start from a template" className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
      <div className="w-full max-w-2xl max-h-[85vh] overflow-auto rounded-lg bg-white dark:bg-gray-900 p-5 space-y-4">
        <div className="flex items-center justify-between">
          <h2 className="text-base font-semibold">Start from a template</h2>
          <button type="button" onClick={onClose} className="text-sm underline cursor-pointer">Close</button>
        </div>
        {error ? <ErrorState error={error} /> : null}
        {!templates && !error ? <Skeleton /> : null}
        {templates && templates.length === 0 ? <EmptyState title="No templates yet" description="An administrator publishes a template, and a second administrator approves it." /> : null}
        {failure ? <p role="alert" className="text-sm text-red-700">{failure}</p> : null}
        {result ? (
          <p role="status" className="text-sm">
            {result.use.approved
              ? `${result.name} is live as ${result.use.app_slug}. The template had been reviewed, so your approval was enough.`
              : raised.length > 0
                ? `${result.name} is waiting for a second person. These checks were raised: ${raised.map((c) => c.name).join(", ")}.`
                : `${result.name} is waiting for approval.`}
          </p>
        ) : null}
        {categories.map((category) => (
          <section key={category} aria-label={category} className="space-y-3">
            <h3 className="text-xs font-semibold uppercase tracking-wide text-gray-500">{category}</h3>
            {(templates ?? []).filter((t) => t.category === category).map((t) => (
              <article key={t.id} className="rounded border border-gray-200 dark:border-gray-700 p-3 space-y-2">
                <h4 className="text-sm font-semibold">{t.name}</h4>
                {t.description ? <p className="text-sm">{t.description}</p> : null}
                <ul className="list-disc pl-5 text-sm space-y-0.5">
                  {t.contents.map((line, i) => <li key={i}>{line}</li>)}
                </ul>
                <p className="text-xs text-gray-500">Needs a workspace classified: {t.needs_classification}</p>
                <button type="button" disabled={busy} onClick={() => use(t)} className="px-3 py-1.5 text-xs font-semibold rounded-md bg-blue-600 text-white cursor-pointer disabled:opacity-50">
                  Use this template
                </button>
              </article>
            ))}
          </section>
        ))}
      </div>
    </div>
  );
}
