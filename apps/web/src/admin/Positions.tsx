import { useCallback, useState, type FormEvent } from "react";
import { apiClient, type PositionTypeRow, type VacancyRow } from "../api";
import {
  ConfirmAction,
  DataTable,
  Drawer,
  ErrorState,
  FormField,
  PageHeader,
  StatusBadge,
  Tabs,
  type DataTableColumn,
} from "../ui";

const ORG_TYPES = ["Institution", "College", "Department", "Center", "Program"];

const inputClass =
  "w-full text-sm rounded border border-gray-300 dark:border-gray-700 p-2 bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100";

export function Positions() {
  const [tab, setTab] = useState<"types" | "vacancies">("types");
  const [creating, setCreating] = useState(false);
  const [version, setVersion] = useState(0);
  const [assigning, setAssigning] = useState<VacancyRow | null>(null);

  const loadTypes = useCallback(async () => {
    const res = await apiClient.adminListPositions();
    return { rows: res.positions };
  }, []);
  const loadVacancies = useCallback(async () => {
    const res = await apiClient.adminPositionVacancies();
    return { rows: res.vacancies };
  }, []);

  const typeColumns: DataTableColumn<PositionTypeRow>[] = [
    { id: "name", header: "Position", accessorKey: "name" },
    { id: "key", header: "Key", accessorKey: "key" },
    { id: "org_types", header: "Applies to", cell: ({ row }) => row.original.org_types.join(", ") },
    { id: "holders", header: "Holders", cell: ({ row }) => `${row.original.holder_count} of ${row.original.max_holders} each` },
    {
      id: "status",
      header: "Status",
      cell: ({ row }) =>
        row.original.retired ? <StatusBadge tone="neutral" text="Retired" /> : <StatusBadge tone="ok" text="In use" />,
    },
  ];

  const vacancyColumns: DataTableColumn<VacancyRow>[] = [
    { id: "unit", header: "Unit", cell: ({ row }) => row.original.unit.name },
    { id: "org_type", header: "Type", accessorKey: "org_type" },
    { id: "position", header: "Position", cell: ({ row }) => row.original.position.name },
    {
      id: "assign",
      header: "",
      cell: ({ row }) => (
        <button
          type="button"
          data-testid={`vacancy-assign-${row.original.unit.id}-${row.original.position.key}`}
          onClick={(e) => {
            e.stopPropagation();
            setAssigning(row.original);
          }}
          className="px-2 py-1 text-xs font-medium rounded border border-gray-300 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-800"
        >
          Assign
        </button>
      ),
    },
  ];

  return (
    <div className="space-y-6" data-testid="admin-positions">
      <PageHeader
        title="Positions"
        description="The posts units can have, such as a department chair, and who holds them"
        actions={
          <button
            type="button"
            data-testid="position-create-open"
            onClick={() => setCreating(true)}
            className="px-3 py-1.5 text-sm font-medium rounded-md bg-blue-600 text-white hover:bg-blue-700"
          >
            New position type
          </button>
        }
      />
      <Tabs
        ariaLabel="Positions"
        selectedId={tab}
        onSelect={(id) => setTab(id as "types" | "vacancies")}
        tabs={[
          { id: "types", label: "Position types" },
          { id: "vacancies", label: "Vacancies" },
        ]}
      />
      {tab === "types" ? (
        <DataTable<PositionTypeRow> key={`t${version}`} columns={typeColumns} load={loadTypes} rowKey="key" />
      ) : (
        <DataTable<VacancyRow>
          key={`v${version}`}
          columns={vacancyColumns}
          load={loadVacancies}
          rowKey={(r) => `${r.unit.id}:${r.position.key}`}
        />
      )}
      <Drawer open={creating} onClose={() => setCreating(false)} title="New position type">
        <CreateForm
          onCreated={() => {
            setCreating(false);
            setVersion((v) => v + 1);
          }}
        />
      </Drawer>
      <Drawer open={assigning !== null} onClose={() => setAssigning(null)} title="Assign a holder">
        {assigning && (
          <AssignForm
            vacancy={assigning}
            onDone={() => {
              setAssigning(null);
              setVersion((v) => v + 1);
            }}
          />
        )}
      </Drawer>
    </div>
  );
}

function CreateForm({ onCreated }: { onCreated: () => void }) {
  const [key, setKey] = useState("");
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [orgTypes, setOrgTypes] = useState<string[]>([]);
  const [maxHolders, setMaxHolders] = useState(1);
  const [reason, setReason] = useState("");
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const valid = key.trim() !== "" && name.trim() !== "" && orgTypes.length > 0 && reason.trim() !== "";

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!valid || busy) return;
    setBusy(true);
    setError(null);
    try {
      await apiClient.adminCreatePosition({
        key: key.trim(),
        name: name.trim(),
        description,
        org_types: orgTypes,
        max_holders: maxHolders,
        reason,
      });
      onCreated();
    } catch (err: unknown) {
      setError(err);
    } finally {
      setBusy(false);
    }
  };

  return (
    <form data-testid="position-create-form" onSubmit={submit} className="space-y-4">
      {error !== null && <ErrorState error={error} />}
      <FormField id="position-key" label="Key" hint="Lowercase letters, digits, - and _" required>
        {(props) => <input {...props} className={inputClass} value={key} onChange={(e) => setKey(e.target.value)} />}
      </FormField>
      <FormField id="position-name" label="Name" required>
        {(props) => <input {...props} className={inputClass} value={name} onChange={(e) => setName(e.target.value)} />}
      </FormField>
      <FormField id="position-description" label="Description">
        {(props) => (
          <input {...props} className={inputClass} value={description} onChange={(e) => setDescription(e.target.value)} />
        )}
      </FormField>
      <fieldset className="space-y-1">
        <legend className="text-sm font-medium text-gray-700 dark:text-gray-300">Applies to</legend>
        {ORG_TYPES.map((t) => (
          <label key={t} className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={orgTypes.includes(t)}
              onChange={(e) => setOrgTypes((cur) => (e.target.checked ? [...cur, t] : cur.filter((x) => x !== t)))}
            />
            {t}
          </label>
        ))}
      </fieldset>
      <FormField id="position-max" label="Holders allowed" hint="1 to 50">
        {(props) => (
          <input
            {...props}
            type="number"
            min={1}
            max={50}
            className={inputClass}
            value={maxHolders}
            onChange={(e) => setMaxHolders(Number(e.target.value))}
          />
        )}
      </FormField>
      <FormField id="position-reason" label="Reason" required>
        {(props) => <input {...props} className={inputClass} value={reason} onChange={(e) => setReason(e.target.value)} />}
      </FormField>
      <div className="flex justify-end">
        <button
          type="submit"
          disabled={!valid || busy}
          className="px-4 py-2 text-sm font-medium rounded-md bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-50"
        >
          Create position type
        </button>
      </div>
    </form>
  );
}

function AssignForm({ vacancy, onDone }: { vacancy: VacancyRow; onDone: () => void }) {
  const [eppn, setEppn] = useState("");
  return (
    <div className="space-y-4" data-testid="vacancy-assign-form">
      <p className="text-sm text-gray-700 dark:text-gray-300">
        {vacancy.position.name} at {vacancy.unit.name} is vacant.
      </p>
      <FormField id="vacancy-eppn" label="Person (user name)" required>
        {(props) => <input {...props} className={inputClass} value={eppn} onChange={(e) => setEppn(e.target.value)} />}
      </FormField>
      {eppn.trim() !== "" && (
        <ConfirmAction
          verb="Confirm"
          target="assignment"
          consequence={`${eppn.trim()} becomes the ${vacancy.position.name} of ${vacancy.unit.name}.`}
          onConfirm={async (reason) => {
            await apiClient.assignPositionHolder(vacancy.unit.id, vacancy.position.key, {
              eppn: eppn.trim(),
              replace: false,
              reason,
            });
          }}
          onSuccess={onDone}
        />
      )}
    </div>
  );
}
