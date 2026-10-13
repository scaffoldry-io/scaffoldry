import { unsavedViewAddress } from "./address";

/** What running a summary view returns: its columns, and a row for each group with the keys and then one value for each measure. */
export interface Summary {
  columns: string[];
  groups: { key: string[]; values: (number | null)[] }[];
  has_more: boolean;
}

export interface SummaryTableProps {
  summary: Summary;
  appSlug: string;
  tableId: string;
  /** The fields the summary groups by, in order. */
  groupBy: string[];
  /** Where the grid lives. The group goes on the end as an address. */
  gridPath: string;
}

/** The most groups a summary returns. */
export const SUMMARY_LIMIT = 200;

const show = (v: number | null) => (v === null ? "" : v.toLocaleString("en-US", { maximumFractionDigits: 2 }));

/**
 * A read-only table of groups and their measures. A row is a link: it opens the grid on that group, in an unsaved view whose
 * filter is in the address. A group with no value reads `(blank)`.
 */
export function SummaryTable({ summary, appSlug, tableId, groupBy, gridPath }: SummaryTableProps) {
  return (
    <div>
      <table aria-label="Summary" className="w-full text-sm border-collapse">
        <thead>
          <tr>
            {summary.columns.map((c) => (
              <th key={c} scope="col" className="text-left border-b px-2 py-1">
                {c}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {summary.groups.map((g) => {
            const href = `${gridPath}${unsavedViewAddress(appSlug, tableId, g.key.map((value, i) => ({ field: groupBy[i], value })))}`;
            return (
              <tr key={g.key.join("\u0000")}>
                {g.key.map((k, i) => (
                  <td key={i} className="border-b px-2 py-1">
                    {i === 0 ? (
                      <a href={href} className="underline">
                        {k === "" ? "(blank)" : k}
                      </a>
                    ) : k === "" ? (
                      "(blank)"
                    ) : (
                      k
                    )}
                  </td>
                ))}
                {g.values.map((v, i) => (
                  <td key={i} className="border-b px-2 py-1 tabular-nums">
                    {show(v)}
                  </td>
                ))}
              </tr>
            );
          })}
        </tbody>
      </table>
      {summary.has_more && (
        <p role="status" className="mt-2 text-sm text-gray-700 dark:text-gray-300">
          Showing the first {SUMMARY_LIMIT} groups. There are more.
        </p>
      )}
    </div>
  );
}
