import { useEffect, useState } from "react";
import { apiClient, type NotificationPrefs } from "./api";
import { ErrorState } from "./ui";

/** The channels in words. */
export const CHANNEL_WORDS: [string, string][] = [
  ["in_app", "In the inbox"],
  ["email", "By mail only"],
  ["both", "Inbox and mail"],
  ["digest", "In a morning digest"],
  ["off", "Off"],
];

function useNotificationPrefs() {
  const [data, setData] = useState<NotificationPrefs | null>(null);
  const [error, setError] = useState<unknown>(null);
  useEffect(() => {
    apiClient.getNotificationPrefs().then(setData).catch(setError);
  }, []);
  return { data, setData, error, setError };
}

/**
 * The `Notifications` section of the settings pane (notifications phase 3): each kind in plain words with a channel
 * choice. Kinds that can never be turned below the inbox are shown fixed, with the reason. A line says whether the
 * institution sends mail at all.
 */
export function NotificationPrefsPanel() {
  const { data, setData, error, setError } = useNotificationPrefs();

  const choose = async (kind: string, channel: string) => {
    try {
      await apiClient.setNotificationPref(kind, channel);
      setData((d) => (d ? { ...d, kinds: d.kinds.map((k) => (k.kind === kind ? { ...k, channel, chosen: true } : k)) } : d));
    } catch (e) {
      setError(e);
    }
  };

  return (
    <section className="space-y-2" aria-labelledby="notification-prefs-heading">
      <h3 id="notification-prefs-heading" className="text-xs font-semibold text-slate-900 dark:text-white uppercase tracking-wider">
        Notifications
      </h3>
      {error !== null && <ErrorState error={error} />}
      {data && (
        <>
          <p className="text-xs text-slate-600 dark:text-slate-300">
            {data.mail_enabled ? "Your institution sends mail." : "Your institution does not send mail, so every notice stays in your inbox."}
          </p>
          <table className="w-full text-xs">
            <tbody>
              {data.kinds.map((k) => (
                <tr key={k.kind} className="border-t border-slate-100 dark:border-slate-800">
                  <td className="py-1 pr-2">{k.label}</td>
                  <td className="py-1">
                    {k.forced ? (
                      <span>
                        Always in your inbox. <span className="text-slate-500">{k.forced_reason}</span>
                      </span>
                    ) : (
                      <select aria-label={k.label} value={k.channel} onChange={(e) => void choose(k.kind, e.target.value)} className="rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 px-1 py-0.5">
                        {CHANNEL_WORDS.map(([value, words]) => (
                          <option key={value} value={value}>
                            {words}
                          </option>
                        ))}
                      </select>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
    </section>
  );
}

/**
 * `Notifications` under Settings in the console: the institution's default channel per kind, which applies to anyone
 * who has not chosen. A forced kind is never offered `Off`. Saving writes the setting `notifications.defaults`, which
 * the server checks and records in the ledger.
 */
export function NotificationDefaultsEditor() {
  const { data, error, setError } = useNotificationPrefs();
  const [values, setValues] = useState<Record<string, string>>({});
  const [saved, setSaved] = useState(false);
  useEffect(() => {
    if (data) setValues(Object.fromEntries(data.kinds.map((k) => [k.kind, k.default])));
  }, [data]);

  const save = async () => {
    setSaved(false);
    try {
      await apiClient.updateSetting("notifications.defaults", values);
      setSaved(true);
    } catch (e) {
      setError(e);
    }
  };

  return (
    <section className="space-y-2" aria-labelledby="notification-defaults-heading">
      <h3 id="notification-defaults-heading" className="text-sm font-semibold text-slate-900 dark:text-white">
        Notifications
      </h3>
      <p className="text-xs text-slate-500">The channel each kind uses for anyone who has not chosen their own.</p>
      {error !== null && <ErrorState error={error} />}
      {data && (
        <table className="w-full text-xs">
          <tbody>
            {data.kinds.map((k) => (
              <tr key={k.kind} className="border-t border-slate-100 dark:border-slate-800">
                <td className="py-1 pr-2">{k.label}</td>
                <td className="py-1">
                  <select aria-label={k.label} value={values[k.kind] ?? k.default} onChange={(e) => setValues((v) => ({ ...v, [k.kind]: e.target.value }))} className="rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 px-1 py-0.5">
                    {CHANNEL_WORDS.filter(([value]) => !(k.forced && value === "off")).map(([value, words]) => (
                      <option key={value} value={value}>
                        {words}
                      </option>
                    ))}
                  </select>
                  {k.forced && <span className="ml-2 text-slate-500">Cannot be off.</span>}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <div className="flex items-center gap-2">
        <button type="button" onClick={() => void save()} className="px-2 py-1 rounded bg-blue-600 text-white text-xs font-medium">
          Save defaults
        </button>
        {saved && <span className="text-xs text-emerald-700">Saved.</span>}
      </div>
    </section>
  );
}
