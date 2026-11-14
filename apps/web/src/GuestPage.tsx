import { useCallback, useEffect, useState, type FormEvent } from "react";
import { ApiError, guestApi, guestSession, type GuestField, type GuestOverview, type GuestRecord, type GuestSession } from "./api";

export type GuestRoute =
  | { kind: "link"; token: string }
  | { kind: "page"; app: string; form: string }
  | { kind: "stop"; token: string };

const typeName = (t: unknown): string => (typeof t === "string" ? t : t && typeof t === "object" ? Object.keys(t as object)[0] ?? "" : "");

/** A stored value in words: an option shows its label, a list its items, a missing value a dash. */
function shown(field: GuestField, value: unknown): string {
  if (value === null || value === undefined || value === "") return "—";
  const label = (v: unknown) => field.options.find((o) => o.id === v)?.label ?? String(v);
  if (Array.isArray(value)) return value.map(label).join(", ");
  if (typeof value === "boolean") return value ? "Yes" : "No";
  return label(value);
}

function Input({ field, value, onChange }: { field: GuestField; value: unknown; onChange: (v: unknown) => void }) {
  const id = `guest-${field.name}`;
  const kind = typeName(field.type);
  const common = { id, name: field.name, required: field.required };
  if (kind === "Select") {
    return (
      <select {...common} value={String(value ?? "")} onChange={(e) => onChange(e.target.value === "" ? null : e.target.value)} className="w-full rounded border px-2 py-1.5 text-sm">
        <option value="">Choose…</option>
        {field.options.map((o) => (
          <option key={o.id} value={o.id}>
            {o.label}
          </option>
        ))}
      </select>
    );
  }
  if (kind === "LongText") {
    return <textarea {...common} value={String(value ?? "")} onChange={(e) => onChange(e.target.value)} rows={4} className="w-full rounded border px-2 py-1.5 text-sm" />;
  }
  if (kind === "Boolean" || kind === "Checkbox") {
    return <input {...common} type="checkbox" checked={value === true} onChange={(e) => onChange(e.target.checked)} />;
  }
  if (["Number", "Currency", "Percent", "Rating"].includes(kind)) {
    return <input {...common} type="number" value={String(value ?? "")} onChange={(e) => onChange(e.target.value === "" ? null : Number(e.target.value))} className="w-full rounded border px-2 py-1.5 text-sm" />;
  }
  return <input {...common} type={kind === "Date" ? "date" : kind === "Email" ? "email" : "text"} value={String(value ?? "")} onChange={(e) => onChange(e.target.value)} className="w-full rounded border px-2 py-1.5 text-sm" />;
}

function RecordForm({ name, fields, start, onSave, onCancel }: { name: string; fields: GuestField[]; start: Record<string, unknown>; onSave: (data: Record<string, unknown>) => Promise<void>; onCancel: () => void }) {
  const [data, setData] = useState<Record<string, unknown>>(start);
  const [problem, setProblem] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setProblem(null);
    const clean: Record<string, unknown> = {};
    for (const f of fields) if (data[f.name] !== undefined && data[f.name] !== null && data[f.name] !== "") clean[f.name] = data[f.name];
    try {
      await onSave(clean);
    } catch (err) {
      setProblem(err instanceof Error ? err.message : "That could not be saved.");
      setBusy(false);
    }
  };
  return (
    <form aria-label={name} onSubmit={submit} className="space-y-3 rounded border p-3">
      {fields.map((f) => (
        <div key={f.name}>
          <label htmlFor={`guest-${f.name}`} className="block text-sm font-medium">
            {f.label}
            {f.required ? " (required)" : ""}
          </label>
          <Input field={f} value={data[f.name]} onChange={(v) => setData((d) => ({ ...d, [f.name]: v }))} />
        </div>
      ))}
      {problem && (
        <p role="alert" className="text-sm text-red-700">
          {problem}
        </p>
      )}
      <div className="flex gap-2">
        <button type="submit" disabled={busy} className="rounded bg-slate-900 px-3 py-1.5 text-sm font-medium text-white disabled:opacity-50">
          Save
        </button>
        <button type="button" onClick={onCancel} className="rounded border px-3 py-1.5 text-sm font-medium">
          Cancel
        </button>
      </div>
    </form>
  );
}

function SignIn({ app, form, note }: { app: string; form: string; note?: string }) {
  const [email, setEmail] = useState("");
  const [honey, setHoney] = useState("");
  const [answer, setAnswer] = useState<string | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const send = async (e: FormEvent) => {
    e.preventDefault();
    setProblem(null);
    try {
      setAnswer((await guestApi.start(app, form, email.trim(), honey)).message);
    } catch (err) {
      setProblem(err instanceof Error ? err.message : "That could not be sent.");
    }
  };
  return (
    <main className="mx-auto max-w-md space-y-3 p-6">
      <h1 className="text-lg font-semibold">Sign in</h1>
      {note && <p className="text-sm text-slate-600">{note}</p>}
      <p className="text-sm text-slate-600">Enter your email address. We will send you a link that works once.</p>
      <form aria-label="Sign in" onSubmit={send} className="space-y-3">
        <div>
          <label htmlFor="guest-email" className="block text-sm font-medium">
            Email address
          </label>
          <input id="guest-email" type="email" required value={email} onChange={(e) => setEmail(e.target.value)} className="w-full rounded border px-2 py-1.5 text-sm" />
        </div>
        {/* A field people never see. A bot that fills it gets the same answer, and nothing happens. */}
        <div aria-hidden="true" style={{ position: "absolute", left: "-10000px", height: 0, overflow: "hidden" }}>
          <input name="website" type="text" tabIndex={-1} autoComplete="off" value={honey} onChange={(e) => setHoney(e.target.value)} />
        </div>
        <button type="submit" className="rounded bg-slate-900 px-3 py-1.5 text-sm font-medium text-white">
          Send me a link
        </button>
      </form>
      {answer && <p role="status" className="text-sm">{answer}</p>}
      {problem && <p role="alert" className="text-sm text-red-700">{problem}</p>}
    </main>
  );
}

function StatusPage({ session, onEnd }: { session: GuestSession; onEnd: () => void }) {
  const [data, setData] = useState<GuestOverview | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [editing, setEditing] = useState<string | "new" | null>(null);
  const [thanks, setThanks] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setData(await guestApi.overview(session));
    } catch (e) {
      if (e instanceof ApiError && e.status === 401) {
        onEnd();
        return;
      }
      setProblem(e instanceof Error ? e.message : "This page could not be opened.");
    }
  }, [session, onEnd]);

  useEffect(() => {
    void load();
  }, [load]);

  if (problem) return <p role="alert" className="p-6 text-sm text-red-700">{problem}</p>;
  if (!data) return <p className="p-6 text-sm text-slate-500">Loading…</p>;
  const fields = data.form.fields.filter((f) => !f.read_only);
  const value = (r: GuestRecord) => Object.fromEntries(fields.map((f) => [f.name, r.data[f.name]]));

  return (
    <main className="mx-auto max-w-2xl space-y-4 p-6">
      <header className="flex items-start justify-between gap-3">
        <h1 className="text-lg font-semibold">{data.form.name}</h1>
        <button type="button" onClick={onEnd} className="rounded border px-3 py-1.5 text-sm">
          Sign out
        </button>
      </header>
      {thanks && <p role="status" className="rounded bg-emerald-50 p-2 text-sm text-emerald-900">{thanks}</p>}
      {data.records.length === 0 && <p className="text-sm text-slate-600">You have not sent anything yet.</p>}
      <ul className="space-y-3">
        {data.records.map((r, i) => (
          <li key={r.id} className="space-y-2 rounded border p-3">
            {editing === r.id ? (
              <RecordForm
                name="Change this request"
                fields={fields}
                start={value(r)}
                onCancel={() => setEditing(null)}
                onSave={async (d) => {
                  await guestApi.update(session, r.id, r.version, d);
                  setEditing(null);
                  setThanks(null);
                  await load();
                }}
              />
            ) : (
              <>
                <p className="text-sm font-medium">{r.standing}</p>
                <dl className="grid grid-cols-[auto,1fr] gap-x-3 gap-y-1 text-sm">
                  {data.form.fields.map((f) => (
                    <div key={f.name} className="contents">
                      <dt className="text-slate-600">{f.label}</dt>
                      <dd>{shown(f, r.data[f.name])}</dd>
                    </div>
                  ))}
                </dl>
                {r.editable && (
                  <button type="button" aria-label={`Edit request ${i + 1}`} onClick={() => setEditing(r.id)} className="rounded border px-3 py-1.5 text-sm font-medium">
                    Edit
                  </button>
                )}
              </>
            )}
          </li>
        ))}
      </ul>
      {editing === "new" ? (
        <RecordForm
          name="Add another"
          fields={fields}
          start={{}}
          onCancel={() => setEditing(null)}
          onSave={async (d) => {
            const made = await guestApi.create(session, d);
            setEditing(null);
            setThanks(made.success_message ?? data.form.success_message ?? "Thank you. Your answers were saved.");
            await load();
          }}
        />
      ) : data.can_add ? (
        <button type="button" onClick={() => setEditing("new")} className="rounded bg-slate-900 px-3 py-1.5 text-sm font-medium text-white">
          Add another
        </button>
      ) : (
        <p className="text-sm text-slate-600">You have sent the most this form takes for now.</p>
      )}
    </main>
  );
}

/**
 * The guest's whole product. A link trades for a session, the page lists what the guest sent, and a mail's last link stops mail.
 * It is deliberately plain and has no navigation, no menu and no link, so nothing in it leads to the rest of the product.
 */
export function GuestPage({ route }: { route: GuestRoute }) {
  const [session, setSession] = useState<GuestSession | null>(() => {
    if (route.kind !== "page") return null;
    const s = guestSession.read();
    return s && s.app === route.app && s.form === route.form ? s : null;
  });
  const [problem, setProblem] = useState<string | null>(null);
  const [stopped, setStopped] = useState<string | null>(null);
  const [target, setTarget] = useState<{ app: string; form: string } | null>(route.kind === "page" ? { app: route.app, form: route.form } : null);

  useEffect(() => {
    if (route.kind !== "link") return;
    let live = true;
    guestApi
      .verify(route.token)
      .then((made) => {
        if (!live) return;
        const s = { token: made.token, app: made.app, form: made.form };
        guestSession.write(s);
        window.history.replaceState(null, "", `/g/${made.app}/${made.form}`);
        setTarget({ app: made.app, form: made.form });
        setSession(s);
      })
      .catch((e: unknown) => live && setProblem(e instanceof Error ? e.message : "This link does not work."));
    return () => {
      live = false;
    };
  }, [route]);

  const end = useCallback(() => {
    guestSession.clear();
    setSession(null);
  }, []);

  if (route.kind === "stop") {
    return (
      <main className="mx-auto max-w-md space-y-3 p-6">
        <h1 className="text-lg font-semibold">Mail about your requests</h1>
        {stopped ? (
          <p role="status" className="text-sm">{stopped}</p>
        ) : (
          <button
            type="button"
            className="rounded bg-slate-900 px-3 py-1.5 text-sm font-medium text-white"
            onClick={() => guestApi.stop(route.token).then((r) => setStopped(r.message), (e: unknown) => setProblem(e instanceof Error ? e.message : "That did not work."))}
          >
            Stop these mails
          </button>
        )}
        {problem && <p role="alert" className="text-sm text-red-700">{problem}</p>}
      </main>
    );
  }
  if (problem) return <p role="alert" className="p-6 text-sm text-red-700">{problem}</p>;
  if (route.kind === "link" && !session) return <p className="p-6 text-sm text-slate-500">Opening your link…</p>;
  if (session) return <StatusPage session={session} onEnd={end} />;
  return target ? <SignIn app={target.app} form={target.form} /> : null;
}
