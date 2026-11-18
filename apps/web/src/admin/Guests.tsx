import { useCallback, useEffect, useState } from "react";
import { apiClient, type AdminGuests } from "../api";
import { ConfirmAction, EmptyState, ErrorState, PageHeader, Skeleton } from "../ui";

const buttonClass = "px-3 py-1.5 text-sm font-medium rounded border border-gray-300 dark:border-gray-700 disabled:opacity-50";

function when(iso: string | null): string {
  if (!iso) return "never";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString("en-US", { dateStyle: "medium", timeStyle: "short" });
}

type Act =
  | { kind: "form"; id: string; name: string; close: boolean }
  | { kind: "guest"; id: string; name: string; action: "disable" | "enable" | "erase" };

// The Guests section of the console (guests phase 4): the forms open to people outside the institution, the guests, and the
// signs of abuse. Every change asks for a reason and is a ledger entry.
export function Guests() {
  const [data, setData] = useState<AdminGuests | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [act, setAct] = useState<Act | null>(null);

  const load = useCallback(async () => {
    setError(null);
    try {
      setData(await apiClient.adminGuests());
    } catch (e) {
      setError(e);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  if (error !== null) return <ErrorState error={error} />;
  if (data === null) return <Skeleton rows={3} />;

  const confirmation = () => {
    if (!act) return null;
    const done = () => {
      setAct(null);
      void load();
    };
    if (act.kind === "form") {
      return (
        <ConfirmAction
          verb={act.close ? "Close" : "Open"}
          target={act.name}
          consequence={
            act.close
              ? "New link requests and submissions are refused from the next request, and existing records are kept. Guests who are signed in see that the form is not accepting responses."
              : "Guests can ask for a link and send answers again."
          }
          onConfirm={async (reason) => {
            await (act.close ? apiClient.closeGuestForm(act.id, reason) : apiClient.openGuestForm(act.id, reason));
          }}
          onSuccess={done}
        />
      );
    }
    const words = {
      disable: { verb: "Disable", consequence: "This guest's session stops on its next request, and no link can be used for them." },
      enable: { verb: "Enable", consequence: "This guest can sign in again." },
      erase: {
        verb: "Erase",
        consequence: "The address is replaced by a hash, mail that has not gone is removed, and the person is signed out for good. Their records stay and show Former applicant. This cannot be undone.",
      },
    }[act.action];
    return (
      <ConfirmAction
        verb={words.verb}
        target={act.name}
        consequence={words.consequence}
        onConfirm={async (reason) => {
          await apiClient.actOnGuest(act.id, act.action, reason);
        }}
        onSuccess={done}
      />
    );
  };

  return (
    <div className="space-y-6">
      <PageHeader title="Guests" description="People outside the institution who fill in a form. A guest sees only what they made, and only the fields the form lists." />

      <section className="space-y-2" aria-labelledby="guest-forms">
        <h3 id="guest-forms" className="text-sm font-semibold">Forms open to guests</h3>
        {data.forms.length === 0 ? (
          <EmptyState title="No form is open to guests" description="An owner or admin proposes a form, and a second person approves it." />
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-sm" aria-label="Forms open to guests">
              <thead>
                <tr className="text-left text-xs text-gray-500">
                  <th className="pr-3 py-1">App</th>
                  <th className="pr-3">Form</th>
                  <th className="pr-3">Submissions</th>
                  <th className="pr-3">Last submission</th>
                  <th className="pr-3">Link requests today</th>
                  <th className="pr-3">Rate-limit hits</th>
                  <th className="pr-3">State</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {data.forms.map((f) => (
                  <tr key={f.form_id} className="border-t border-gray-200 dark:border-gray-800">
                    <td className="pr-3 py-1">{f.app}</td>
                    <td className="pr-3">{f.name}</td>
                    <td className="pr-3">{f.submissions}</td>
                    <td className="pr-3 text-xs">{when(f.last_submission)}</td>
                    <td className="pr-3">{f.link_requests_today}</td>
                    <td className="pr-3">{f.rate_limit_hits}</td>
                    <td className="pr-3 font-semibold">{f.closed ? "Closed" : "Open"}</td>
                    <td>
                      <button type="button" className={buttonClass} aria-label={`${f.closed ? "Open" : "Close"} ${f.name}`} onClick={() => setAct({ kind: "form", id: f.form_id, name: f.name, close: !f.closed })}>
                        {f.closed ? "Open this form" : "Close this form"}
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>

      <section className="space-y-2" aria-labelledby="guest-list">
        <h3 id="guest-list" className="text-sm font-semibold">Guests</h3>
        {data.guests.length === 0 ? (
          <EmptyState title="No guests yet" description="A guest is made when someone asks for a link to a form." />
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-sm" aria-label="Guests">
              <thead>
                <tr className="text-left text-xs text-gray-500">
                  <th className="pr-3 py-1">Address</th>
                  <th className="pr-3">Forms used</th>
                  <th className="pr-3">Submissions</th>
                  <th className="pr-3">Last seen</th>
                  <th className="pr-3">State</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {data.guests.map((g) => (
                  <tr key={g.id} className="border-t border-gray-200 dark:border-gray-800">
                    <td className="pr-3 py-1">{g.address}</td>
                    <td className="pr-3">{g.forms_used}</td>
                    <td className="pr-3">{g.submissions}</td>
                    <td className="pr-3 text-xs">{when(g.last_seen)}</td>
                    <td className="pr-3 font-semibold">{g.state}</td>
                    <td className="space-x-2 whitespace-nowrap">
                      {g.state !== "Erased" && (
                        <>
                          <button type="button" className={buttonClass} aria-label={`${g.state === "Disabled" ? "Enable" : "Disable"} ${g.address}`} onClick={() => setAct({ kind: "guest", id: g.id, name: g.address, action: g.state === "Disabled" ? "enable" : "disable" })}>
                            {g.state === "Disabled" ? "Enable" : "Disable"}
                          </button>
                          <button type="button" className={buttonClass} aria-label={`Erase ${g.address}`} onClick={() => setAct({ kind: "guest", id: g.id, name: g.address, action: "erase" })}>
                            Erase
                          </button>
                        </>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>

      {act && <div className="rounded border p-3">{confirmation()}</div>}

      <section className="space-y-2" aria-labelledby="guest-abuse">
        <h3 id="guest-abuse" className="text-sm font-semibold">Signs of abuse</h3>
        {data.abuse.bounces.length === 0 && data.abuse.networks.length === 0 ? (
          <p className="text-sm text-gray-600">Nothing to show. No address has bounced three times, and no network has hit a limit in the last day.</p>
        ) : (
          <div className="grid gap-4 md:grid-cols-2">
            <div>
              <h4 className="text-xs font-medium text-gray-500">Addresses with repeated bounces</h4>
              {data.abuse.bounces.length === 0 ? (
                <p className="text-sm text-gray-600">None.</p>
              ) : (
                <ul className="text-sm">
                  {data.abuse.bounces.map((b) => (
                    <li key={b.address}>{`${b.address}: ${b.bounces} failed mails`}</li>
                  ))}
                </ul>
              )}
            </div>
            <div>
              <h4 className="text-xs font-medium text-gray-500">Networks that hit a limit in the last day</h4>
              {data.abuse.networks.length === 0 ? (
                <p className="text-sm text-gray-600">None.</p>
              ) : (
                <ul className="text-sm">
                  {data.abuse.networks.map((n) => (
                    <li key={n.network}>{`${n.network}: ${n.hits} requests over the limit`}</li>
                  ))}
                </ul>
              )}
            </div>
          </div>
        )}
      </section>
    </div>
  );
}
