import { useCallback, useState } from "react";
import { apiClient, type UnitPositionRow } from "../api";
import { ConfirmAction, ErrorState, PersonLabel, Skeleton, useAsync } from "../ui";

type Action =
  | { kind: "assign" | "replace"; key: string; name: string }
  | { kind: "vacate"; key: string; name: string; eppn: string };

const inputClass =
  "w-full text-sm rounded border border-gray-300 dark:border-gray-700 p-2 bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100";

/** The positions a unit can have, who holds each, and the actions on them. */
export function UnitPositions({ unitId }: { unitId: string }) {
  const load = useCallback(() => apiClient.listUnitPositions(unitId), [unitId]);
  const { data, error, loading, reload } = useAsync(load, true);
  const [action, setAction] = useState<Action | null>(null);
  const [eppn, setEppn] = useState("");

  if (loading && !data) return <Skeleton rows={2} />;
  if (error) return <ErrorState error={error} onRetry={() => reload().catch(() => {})} />;
  const positions: UnitPositionRow[] = Array.isArray(data?.positions) ? data!.positions : [];
  if (positions.length === 0) {
    return <p className="text-xs text-gray-500 italic">No position types apply to this unit.</p>;
  }

  const done = () => {
    setAction(null);
    setEppn("");
    reload().catch(() => {});
  };

  const run = async (reason: string): Promise<void> => {
    if (!action) return;
    if (action.kind === "vacate") {
      await apiClient.vacatePositionHolder(unitId, action.key, action.eppn, reason);
    } else {
      await apiClient.assignPositionHolder(unitId, action.key, {
        eppn: eppn.trim(),
        replace: action.kind === "replace",
        reason,
      });
    }
  };

  const small =
    "px-2 py-1 text-xs font-medium rounded border border-gray-300 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-800 text-gray-700 dark:text-gray-300";

  return (
    <div data-testid="unit-positions" className="space-y-3">
      {positions.map((p) => {
        const full = p.holders.length >= p.max_holders;
        const names = Object.fromEntries(p.holders.map((h) => [h.eppn, h.display_name]));
        return (
          <div key={p.key} className="border border-gray-200 dark:border-gray-800 rounded-md p-3 space-y-2">
            <div className="flex items-center justify-between">
              <span className="text-sm font-semibold text-gray-900 dark:text-gray-100">{p.name}</span>
              <button
                type="button"
                data-testid={`position-${full ? "replace" : "assign"}-${p.key}`}
                className={small}
                onClick={() => setAction({ kind: full ? "replace" : "assign", key: p.key, name: p.name })}
              >
                {full ? "Replace" : "Assign"}
              </button>
            </div>
            {p.holders.length === 0 ? (
              <p className="text-xs text-gray-500">Vacant</p>
            ) : (
              <ul className="space-y-1">
                {p.holders.map((h) => (
                  <li key={h.eppn} className="flex items-center justify-between text-xs">
                    <PersonLabel id={h.eppn} lookup={names} />
                    {h.source === "scim" ? (
                      <span className="text-gray-500">Set by the registry</span>
                    ) : (
                      <button
                        type="button"
                        data-testid={`position-vacate-${p.key}-${h.eppn}`}
                        className={small}
                        onClick={() => setAction({ kind: "vacate", key: p.key, name: p.name, eppn: h.eppn })}
                      >
                        Vacate
                      </button>
                    )}
                  </li>
                ))}
              </ul>
            )}
          </div>
        );
      })}

      {action && (
        <div data-testid="position-action" className="border border-gray-200 dark:border-gray-800 rounded-lg p-3 space-y-3">
          {action.kind !== "vacate" && (
            <div className="space-y-1">
              <label htmlFor="position-holder-eppn" className="block text-xs font-medium text-gray-700 dark:text-gray-300">
                Person (user name)
              </label>
              <input
                id="position-holder-eppn"
                className={inputClass}
                value={eppn}
                onChange={(e) => setEppn(e.target.value)}
              />
            </div>
          )}
          {(action.kind === "vacate" || eppn.trim() !== "") && (
            <ConfirmAction
              verb="Confirm"
              target={action.kind}
              consequence={
                action.kind === "vacate"
                  ? `${action.eppn} stops holding ${action.name} here.`
                  : action.kind === "replace"
                    ? `The current holders of ${action.name} are replaced by ${eppn.trim()}.`
                    : `${eppn.trim()} becomes a holder of ${action.name} here.`
              }
              onConfirm={run}
              onSuccess={done}
            />
          )}
        </div>
      )}
    </div>
  );
}
