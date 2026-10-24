import { useEffect, useState } from "react";
import { apiClient, type OAuthConsent as Consent } from "./api";

/** The sign-in request an outside app sent, taken from the address of this page. */
function requestParams(): Record<string, string> {
  return Object.fromEntries(new URLSearchParams(window.location.search));
}

/**
 * The screen a person sees when an outside app asks to sign them in. It names the app, who runs it, the address
 * the person will be sent back to, and what the app may ever use, in sentences. The app's own text is shown as
 * plain text. A request the server refuses is an error here and never a redirect.
 */
export function OAuthConsent() {
  const [params] = useState(requestParams);
  const [consent, setConsent] = useState<Consent | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [access, setAccess] = useState("read");

  const answer = async (decision: "allow" | "deny") => {
    setBusy(true);
    try {
      const { redirect_to } = await apiClient.oauthDecide(params, decision, decision === "allow" && consent?.choices ? access : undefined);
      window.location.assign(redirect_to);
    } catch (e: any) {
      setError(e?.message || "The answer could not be sent.");
      setBusy(false);
    }
  };

  useEffect(() => {
    let cancelled = false;
    apiClient
      .oauthConsent(params)
      .then((c) => {
        if (cancelled) return;
        setConsent(c);
        // The person already agreed to exactly this, so there is nothing to ask.
        if (c.remembered) void answer("allow");
      })
      .catch((e: any) => {
        if (!cancelled) setError(e?.message || "This sign-in request could not be read.");
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (error) {
    return (
      <main className="max-w-lg mx-auto my-16 p-6 rounded-xl border border-rose-200 bg-white text-slate-900" data-testid="oauth-error">
        <h1 className="text-lg font-bold">This sign-in cannot continue</h1>
        <p className="mt-2 text-sm">{error}</p>
        <p className="mt-2 text-xs text-slate-500">Nothing was shared. Go back to the app and try again. If this keeps happening, tell the people who run it.</p>
      </main>
    );
  }
  if (!consent) {
    return <main className="max-w-lg mx-auto my-16 p-6 text-sm text-slate-600">Checking the request…</main>;
  }
  return (
    <main className="max-w-lg mx-auto my-16 p-6 rounded-xl border border-slate-200 bg-white text-slate-900 space-y-4" data-testid="oauth-consent">
      <h1 className="text-lg font-bold">Allow {consent.client.name} to use your Scaffoldry access?</h1>
      <dl className="text-sm grid grid-cols-[8rem_1fr] gap-y-1">
        <dt className="text-slate-500">Run by</dt>
        <dd>{consent.client.operator}</dd>
        <dt className="text-slate-500">You will return to</dt>
        <dd>{consent.client.callback_origin}</dd>
        <dt className="text-slate-500">Signed in as</dt>
        <dd>{consent.user.name || consent.user.eppn}</dd>
      </dl>
      <section aria-labelledby="oauth-ceiling">
        <h2 id="oauth-ceiling" className="text-xs font-semibold uppercase tracking-wide text-slate-500">
          It may only
        </h2>
        <ul className="mt-1 list-disc pl-5 text-sm space-y-1">
          {consent.sentences.map((s) => (
            <li key={s}>{s}</li>
          ))}
        </ul>
        <p className="mt-2 text-xs text-slate-500">It can never do more than you can. You can disconnect it at any time from your settings.</p>
      </section>
      {consent.choices && (
        <fieldset className="space-y-1" data-testid="oauth-choices">
          <legend className="text-xs font-semibold uppercase tracking-wide text-slate-500">What may it do for you?</legend>
          {consent.choices.map((c) => (
            <label key={c.id} className="flex items-center gap-2 text-sm">
              <input type="radio" name="access" value={c.id} checked={access === c.id} onChange={() => setAccess(c.id)} />
              {c.sentence}
            </label>
          ))}
          <p className="text-xs text-slate-500">
            The session lasts up to {consent.session_hours ?? 12} hours. It also ends when you disconnect it or an administrator turns it off.
          </p>
        </fieldset>
      )}
      <div className="flex gap-2">
        <button type="button" disabled={busy} onClick={() => void answer("allow")} className="px-4 py-2 rounded bg-blue-600 text-white text-sm font-semibold disabled:opacity-50">
          Allow
        </button>
        <button type="button" disabled={busy} onClick={() => void answer("deny")} className="px-4 py-2 rounded border border-slate-300 text-sm font-semibold disabled:opacity-50">
          Deny
        </button>
      </div>
    </main>
  );
}
