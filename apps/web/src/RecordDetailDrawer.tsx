import { DurationEditor } from "./DurationEditor";
import { formatDateTime, parseDateTimeInput, getUserTimezone } from "./utils/durationDatetime";
import React from "react";
import { AppTable } from "./types";
import { LongTextEditor } from "./LongTextEditor";
import { AttachmentField, type AttachedFile } from "./ui/Attachments";
import { computeFieldValue, getFieldTypeIcon } from "./computedFields";
import { LinkedRecords } from "./LinkedRecords";
import { apiClient, type DisclosureEntry } from "./api";
import { RecordHistory } from "./RecordHistory";
import { RecordComments } from "./RecordComments";
import { Tabs, EmptyState, ErrorState, Skeleton } from "./ui";

export interface RecordDetailDrawerProps {
  selectedRecordId: string | null;
  onClose: () => void;
  activeTable: AppTable;
  tables: AppTable[];
  /** The app the records belong to. Attachments need it to upload and download. */
  appSlug?: string;
  setTables: React.Dispatch<React.SetStateAction<AppTable[]>>;
  onUpdateRecordField: (recordId: string, fieldName: string, value: any) => void;
  onDuplicateRecord: (recordId: string) => void;
  onDeleteRecord: (recordId: string) => void;
  /** Opens another record's drawer, from a linked records section. */
  onOpenRecord?: (tableId: string, recordId: string) => void;
  /** Who is looking. It decides which tabs show and what they offer. Without it, the drawer shows Details only. */
  viewer?: { eppn: string; role: "owner" | "admin" | "editor" | "commenter" | "viewer"; compliance?: boolean };
}

export const RecordDetailDrawer: React.FC<RecordDetailDrawerProps> = ({
  selectedRecordId,
  onClose,
  activeTable,
  tables,
  appSlug,
  setTables,
  onUpdateRecordField,
  onDuplicateRecord,
  onDeleteRecord,
  onOpenRecord,
  viewer,
}) => {
  if (!selectedRecordId) return null;
  const activeDetailRecord = (activeTable.records || []).find(
    (r) => r.id === selectedRecordId
  );
  const [editingFields, setEditingFields] = React.useState<Record<string, boolean>>({});
  const [tab, setTab] = React.useState("details");
  // Only a Platform Admin or compliance is told about a hold. Anyone else gets nothing back, and sees nothing.
  const [holdMarker, setHoldMarker] = React.useState<string | null>(null);
  React.useEffect(() => {
    let live = true;
    setHoldMarker(null);
    if (appSlug && selectedRecordId) {
      void apiClient.getRecordHold(appSlug, selectedRecordId).then((h) => {
        if (live && h?.held && h.marker) setHoldMarker(h.marker);
      });
    }
    return () => {
      live = false;
    };
  }, [appSlug, selectedRecordId]);
  if (!activeDetailRecord) return null;

  const relatedTables = tables.filter((t) => t.id !== activeTable.id);
  const reverseLinks = relatedTables
    .map((tbl) => {
      const relField = tbl.fields.find(
        (f) =>
          (f.field_type === "Relation" && f.target_table_id === activeTable.id) ||
          f.name === `${activeTable.slug.replace(/s$/, "")}_id` ||
          (f.name.endsWith("_id") && activeTable.slug.includes(f.name.replace(/_id$/, "")))
      );
      if (!relField) return null;
      const linkedRecords = (tbl.records || []).filter(
        (r) => String(r[relField.name]) === String(activeDetailRecord.id)
      );
      return { table: tbl, relField, linkedRecords };
    })
    .filter(Boolean) as {
    table: AppTable;
    relField: any;
    linkedRecords: Record<string, any>[];
  }[];

  return (
    <div
      data-testid="record-detail-drawer"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-xs p-4"
    >
      <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl shadow-2xl max-w-3xl w-full max-h-[90vh] flex flex-col overflow-hidden">
        {/* DRAWER HEADER */}
        <div className="p-5 border-b border-slate-100 dark:border-slate-800 flex items-center justify-between bg-slate-50/70 dark:bg-slate-800/40">
          <div className="space-y-1 flex-1 mr-4">
            <div className="flex items-center gap-2">
              <span
                className="text-xs font-mono px-2 py-0.5 rounded bg-slate-200 dark:bg-slate-700 text-slate-700 dark:text-slate-300 font-bold"
                data-testid="detail-record-id-badge"
              >
                {activeDetailRecord.id}
              </span>
              <span className="text-xs text-slate-400 font-medium">
                in {activeTable.name}
              </span>
            </div>
            <input
              type="text"
              data-testid="detail-primary-title-input"
              value={String(activeDetailRecord[activeTable.primary_field || "name"] || "")}
              onChange={(e) =>
                onUpdateRecordField(
                  activeDetailRecord.id,
                  activeTable.primary_field || "name",
                  e.target.value
                )
              }
              className="text-lg font-bold text-slate-900 dark:text-white bg-transparent border-b border-transparent hover:border-slate-300 focus:border-blue-500 focus:outline-none w-full py-0.5 transition-colors"
              placeholder="Untitled Record"
            />
          </div>
          <div className="flex items-center gap-2">
            <button
              type="button"
              data-testid="detail-duplicate-btn"
              onClick={() => onDuplicateRecord(activeDetailRecord.id)}
              className="p-1.5 text-slate-400 hover:text-emerald-600 rounded-lg border border-slate-200 dark:border-slate-700 hover:bg-slate-100 dark:hover:bg-slate-800 cursor-pointer text-xs"
              title="Duplicate"
            >
              ⧉ Duplicate
            </button>
            <button
              type="button"
              data-testid="detail-delete-btn"
              onClick={() => onDeleteRecord(activeDetailRecord.id)}
              className="p-1.5 text-slate-400 hover:text-red-600 rounded-lg border border-slate-200 dark:border-slate-700 hover:bg-slate-100 dark:hover:bg-slate-800 cursor-pointer text-xs"
              title="Delete"
            >
              🗑 Delete
            </button>
            <button
              type="button"
              data-testid="detail-close-btn"
              onClick={onClose}
              className="w-8 h-8 rounded-lg text-slate-400 hover:text-slate-700 dark:hover:text-white hover:bg-slate-100 dark:hover:bg-slate-800 flex items-center justify-center font-bold cursor-pointer"
            >
              ✕
            </button>
          </div>
        </div>

        {viewer && appSlug && (
          <div className="px-5 pt-3">
            <Tabs
              ariaLabel="Record"
              selectedId={tab}
              onSelect={setTab}
              tabs={[
                { id: "details", label: "Details" },
                { id: "history", label: "History" },
                { id: "comments", label: "Comments" },
                ...(viewer.role === "owner" || viewer.role === "admin" || viewer.compliance ? [{ id: "disclosures", label: "Disclosures" }] : []),
              ]}
            />
          </div>
        )}

        {/* DRAWER CONTENT */}
        <div className="p-6 overflow-y-auto space-y-6 flex-1">
          {holdMarker && (
            <p role="status" className="text-xs font-semibold text-amber-800 dark:text-amber-300 bg-amber-50 dark:bg-amber-950/40 border border-amber-200 dark:border-amber-900 rounded px-2 py-1">
              {holdMarker}
            </p>
          )}
          {viewer && appSlug && tab === "history" && (
            <RecordHistory appSlug={appSlug} recordId={activeDetailRecord.id} fields={activeTable.fields} canRestore={["owner", "admin", "editor"].includes(viewer.role)} />
          )}
          {viewer && appSlug && tab === "comments" && (
            <RecordComments appSlug={appSlug} recordId={activeDetailRecord.id} viewerEppn={viewer.eppn} role={viewer.role} />
          )}
          {viewer && appSlug && tab === "disclosures" && <RecordDisclosures appSlug={appSlug} recordId={activeDetailRecord.id} />}
          {tab === "details" && (
          <>
          {/* FIELDS SECTION */}
          <div className="space-y-4">
            <h4 className="text-xs font-bold font-mono text-slate-400 uppercase tracking-wider">
              Record Properties
            </h4>
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              {activeTable.fields.map((field) => {
                // A link field is shown in the linked records section below.
                if (field.field_type === "Link") return null;
                // An attachment field draws its own frame, so a field the person may not read leaves no trace.
                if (field.field_type === "Attachment") {
                  return appSlug ? (
                    <AttachmentField
                      key={field.name}
                      appSlug={appSlug}
                      recordId={activeDetailRecord.id}
                      fieldName={field.name}
                      label={field.label}
                      value={Array.isArray(activeDetailRecord[field.name]) ? (activeDetailRecord[field.name] as AttachedFile[]) : []}
                      onChange={(next) => onUpdateRecordField(activeDetailRecord.id, field.name, next)}
                    />
                  ) : null;
                }
                const isComputed = [
                  "Formula",
                  "Lookup",
                  "Count",
                  "Rollup",
                ].includes(field.field_type);
                const computedVal = isComputed
                  ? computeFieldValue(field, activeDetailRecord, tables)
                  : activeDetailRecord[field.name];

                return (
                  <div
                    key={field.name}
                    className={`p-3 rounded-xl border border-slate-200 dark:border-slate-800 bg-slate-50/50 dark:bg-slate-800/30 space-y-1.5 ${field.field_type === "LongText" ? "md:col-span-2" : ""}`}
                  >
                    <div className="flex items-center justify-between">
                      <label className="text-xs font-semibold text-slate-700 dark:text-slate-300 flex items-center gap-1.5">
                        <span className="text-slate-400 font-mono text-[11px]">
                          {getFieldTypeIcon(field.field_type)}
                        </span>
                        <span>{field.label}</span>
                      </label>
                      {isComputed && (
                        <span className="text-[10px] font-mono px-1.5 py-0.2 rounded bg-blue-100 dark:bg-blue-900/60 text-blue-700 dark:text-blue-300 font-bold">
                          Calculated
                        </span>
                      )}
                    </div>

                    {isComputed ? (
                      <div
                        data-testid={`detail-computed-${field.name}`}
                        className="px-2.5 py-1.5 rounded-lg bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-700 font-mono text-xs text-slate-800 dark:text-slate-200"
                      >
                        {typeof computedVal === "number"
                          ? computedVal.toLocaleString()
                          : String(computedVal ?? "")}
                      </div>
                    ) : field.field_type === "LongText" ? (
                      <LongTextEditor
                        id={`detail-input-${field.name}`}
                        label={field.label}
                        value={String(activeDetailRecord[field.name] ?? "")}
                        onChange={(val) =>
                          onUpdateRecordField(
                            activeDetailRecord.id,
                            field.name,
                            val
                          )
                        }
                        isEditing={editingFields[field.name] ?? false}
                        onToggleEdit={() =>
                          setEditingFields((prev) => ({
                            ...prev,
                            [field.name]: !prev[field.name],
                          }))
                        }
                      />
                    ) : field.field_type === "Duration" ? (
                      <DurationEditor
                        id={`detail-input-${field.name}`}
                        label={field.label}
                        value={
                          typeof activeDetailRecord[field.name] === "number"
                            ? (activeDetailRecord[field.name] as number)
                            : null
                        }
                        format={field.duration_format || "h:mm"}
                        onChange={(val) =>
                          onUpdateRecordField(
                            activeDetailRecord.id,
                            field.name,
                            val
                          )
                        }
                      />
                    ) : field.field_type === "DateTime" ? (
                      <div className="space-y-1">
                        <input
                          type="datetime-local"
                          data-testid={`detail-input-${field.name}`}
                          defaultValue={
                            activeDetailRecord[field.name]
                              ? formatDateTime(String(activeDetailRecord[field.name])).replace(" ", "T").slice(0, 16)
                              : ""
                          }
                          onBlur={(e) => {
                            const val = e.target.value;
                            if (!val) {
                              onUpdateRecordField(activeDetailRecord.id, field.name, null);
                            } else {
                              const utcInstant = parseDateTimeInput(val);
                              if (utcInstant) {
                                onUpdateRecordField(activeDetailRecord.id, field.name, utcInstant);
                              }
                            }
                          }}
                          className="w-full px-2.5 py-1.5 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 text-xs text-slate-800 dark:text-white"
                        />
                        <div className="flex justify-between items-center text-[10px] text-slate-400">
                          <span>Stored as UTC instant</span>
                          <span className="font-mono">{getUserTimezone()}</span>
                        </div>
                      </div>
                    ) : field.field_type === "Checkbox" ? (
                      <div className="pt-1">
                        <input
                          type="checkbox"
                          data-testid={`detail-input-${field.name}`}
                          checked={Boolean(activeDetailRecord[field.name])}
                          onChange={(e) =>
                            onUpdateRecordField(
                              activeDetailRecord.id,
                              field.name,
                              e.target.checked
                            )
                          }
                          className="rounded border-slate-300 text-blue-600 focus:ring-blue-500 h-4 w-4 cursor-pointer"
                        />
                      </div>
                    ) : field.field_type === "Select" ? (
                      <select
                        data-testid={`detail-input-${field.name}`}
                        value={String(activeDetailRecord[field.name] || "")}
                        onChange={(e) =>
                          onUpdateRecordField(
                            activeDetailRecord.id,
                            field.name,
                            e.target.value
                          )
                        }
                        className="w-full px-2.5 py-1.5 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 text-xs text-slate-800 dark:text-white"
                      >
                        {(field.select_options || [
                          "Under Review",
                          "Approved",
                          "Funded",
                          "Rejected",
                        ]).map((rawOpt) => {
                          const opt = typeof rawOpt === "string" ? { id: rawOpt, label: rawOpt } : rawOpt;
                          return (
                            <option key={opt.id} value={opt.id}>
                              {opt.label}
                            </option>
                          );
                        })}
                      </select>
                    ) : field.field_type === "Rating" ? (
                      <div className="flex items-center gap-1 pt-1">
                        {[1, 2, 3, 4, 5].map((star) => (
                          <button
                            key={star}
                            type="button"
                            data-testid={`detail-rating-${field.name}-${star}`}
                            onClick={() =>
                              onUpdateRecordField(
                                activeDetailRecord.id,
                                field.name,
                                star
                              )
                            }
                            className={`text-base cursor-pointer ${
                              star <= Number(activeDetailRecord[field.name] || 0)
                                ? "text-amber-500"
                                : "text-slate-300 dark:text-slate-600"
                            }`}
                          >
                            ★
                          </button>
                        ))}
                      </div>
                    ) : (
                      <input
                        type={
                          field.field_type === "Number" ||
                          field.field_type === "Currency"
                            ? "number"
                            : "text"
                        }
                        data-testid={`detail-input-${field.name}`}
                        value={activeDetailRecord[field.name] ?? ""}
                        onChange={(e) => {
                          const v =
                            field.field_type === "Number" ||
                            field.field_type === "Currency"
                              ? Number(e.target.value)
                              : e.target.value;
                          onUpdateRecordField(
                            activeDetailRecord.id,
                            field.name,
                            v
                          );
                        }}
                        className="w-full px-2.5 py-1.5 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 text-xs text-slate-800 dark:text-white font-sans"
                      />
                    )}
                  </div>
                );
              })}
            </div>
          </div>

          {/* LINKED RECORDS: every link field of this table, mirror fields included */}
          {activeTable.fields.some((f) => f.field_type === "Link") && (
            <section aria-label="Linked records" className="space-y-4 pt-4 border-t border-slate-200 dark:border-slate-800">
              <h4 className="text-xs font-bold font-mono text-slate-400 uppercase tracking-wider">Linked records</h4>
              {activeTable.fields
                .filter((f) => f.field_type === "Link")
                .map((field) => (
                  <LinkedRecords
                    key={field.name}
                    field={field}
                    target={tables.find((t) => t.id === field.link_target_table)}
                    value={activeDetailRecord[field.name]}
                    onChange={(next) => onUpdateRecordField(activeDetailRecord.id, field.name, next)}
                    onOpen={onOpenRecord}
                  />
                ))}
            </section>
          )}

          {/* REVERSE RELATIONAL SUB-TABLES */}
          <div
            data-testid="reverse-relation-section"
            className="space-y-4 pt-4 border-t border-slate-200 dark:border-slate-800"
          >
            <h4 className="text-xs font-bold font-mono text-slate-400 uppercase tracking-wider">
              Reverse Relational Sub-Tables
            </h4>

            {reverseLinks.length === 0 ? (
              <p className="text-xs text-slate-400 italic">
                No other tables currently reference {activeTable.name}.
              </p>
            ) : (
              <div className="space-y-4">
                {reverseLinks.map(({ table: relTable, relField, linkedRecords }) => (
                  <div
                    key={relTable.id}
                    className="border border-slate-200 dark:border-slate-800 rounded-xl p-4 bg-white dark:bg-slate-900 space-y-3"
                  >
                    <div className="flex items-center justify-between">
                      <div className="flex items-center gap-2">
                        <span>{relTable.icon || "📑"}</span>
                        <span className="font-bold text-xs text-slate-900 dark:text-white">
                          Linked {relTable.name}
                        </span>
                        <span className="text-[10px] font-mono px-2 py-0.2 rounded-full bg-purple-50 text-purple-700 dark:bg-purple-950 dark:text-purple-300 font-bold">
                          {linkedRecords.length} records
                        </span>
                      </div>
                      <button
                        type="button"
                        data-testid={`add-linked-${relTable.id}-btn`}
                        onClick={() => {
                          const newRelId = `REC-${Date.now().toString().slice(-4)}`;
                          const relPrimary = relTable.primary_field || "name";
                          const curPrimary = activeTable.primary_field || "name";
                          const newRelRec: Record<string, any> = {
                            id: newRelId,
                            [relField.name]: activeDetailRecord.id,
                            [relPrimary]: `New ${relTable.name} for ${
                              activeDetailRecord[curPrimary] || "Record"
                            }`,
                          };
                          setTables((prev) =>
                            prev.map((t) =>
                              t.id === relTable.id
                                ? {
                                    ...t,
                                    records: [...(t.records || []), newRelRec],
                                  }
                                : t
                            )
                          );
                        }}
                        className="text-xs font-semibold text-purple-600 dark:text-purple-400 hover:underline cursor-pointer"
                      >
                        + Add Linked {relTable.name}
                      </button>
                    </div>

                    {linkedRecords.length === 0 ? (
                      <p className="text-xs text-slate-400 italic">
                        No linked records yet.
                      </p>
                    ) : (
                      <div className="overflow-x-auto border border-slate-100 dark:border-slate-800 rounded-lg">
                        <table className="w-full text-left text-xs">
                          <thead>
                            <tr className="bg-slate-50 dark:bg-slate-800/60 font-mono text-[10px] text-slate-500">
                              {relTable.fields
                                .filter((f) => f.name !== relField.name)
                                .slice(0, 4)
                                .map((f) => (
                                  <th key={f.name} className="py-1.5 px-3">
                                    {f.label}
                                  </th>
                                ))}
                            </tr>
                          </thead>
                          <tbody className="divide-y divide-slate-100 dark:divide-slate-800">
                            {linkedRecords.map((lr) => (
                              <tr key={lr.id}>
                                {relTable.fields
                                  .filter((f) => f.name !== relField.name)
                                  .slice(0, 4)
                                  .map((f) => (
                                    <td
                                      key={f.name}
                                      className="py-1.5 px-3 text-slate-700 dark:text-slate-300"
                                    >
                                      {f.field_type === "Currency" ||
                                      f.name.includes("amount")
                                        ? `$${Number(lr[f.name] || 0).toLocaleString()}`
                                        : String(lr[f.name] ?? "")}
                                    </td>
                                  ))}
                              </tr>
                            ))}
                          </tbody>
                        </table>
                      </div>
                    )}
                  </div>
                ))}
              </div>
            )}
          </div>
          </>
          )}
        </div>
      </div>
    </div>
  );
};

const DISCLOSURE_VERBS: Record<string, string> = { export: "Exported", download: "Downloaded", agent: "Read by an agent", api: "Read through the API", share: "Shared" };

function dayMonth(at: string): string {
  const d = new Date(at);
  return Number.isNaN(d.getTime()) ? at : `${d.getUTCDate()} ${d.toLocaleString("en-GB", { month: "long", timeZone: "UTC" })}`;
}

/** `Exported 340 records including Student ID, by Dr. Rivera, 3 March.` */
export function disclosureSentence(d: DisclosureEntry): string {
  const verb = DISCLOSURE_VERBS[d.kind] ?? "Released";
  const fields = d.fields.length ? ` including ${d.fields.join(", ")}` : "";
  return `${verb} ${d.record_count.toLocaleString("en-US")} ${d.record_count === 1 ? "record" : "records"}${fields}, by ${d.actor_name || d.actor}, ${dayMonth(d.at)}.`;
}

/** The disclosure log of one record, for owners, admins and compliance. */
function RecordDisclosures({ appSlug, recordId }: { appSlug: string; recordId: string }) {
  const [rows, setRows] = React.useState<DisclosureEntry[] | null>(null);
  const [error, setError] = React.useState<unknown>(null);
  React.useEffect(() => {
    let live = true;
    apiClient
      .getRecordDisclosures(appSlug, recordId)
      .then((r) => live && setRows(r.disclosures))
      .catch((e) => live && setError(e));
    return () => {
      live = false;
    };
  }, [appSlug, recordId]);
  if (error !== null) return <ErrorState error={error} />;
  if (rows === null) return <Skeleton rows={2} />;
  if (rows.length === 0) return <EmptyState title="No disclosures" description="No release of this record's sensitive fields is on the log." />;
  return (
    <ul aria-label="Disclosures" className="space-y-2">
      {rows.map((d) => (
        <li key={d.id} className="text-sm">
          <p>{disclosureSentence(d)}</p>
          {d.recipient && <p className="text-xs text-gray-600 dark:text-gray-400">To {d.recipient}</p>}
        </li>
      ))}
    </ul>
  );
}
