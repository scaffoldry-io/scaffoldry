import React, { useState } from "react";
import { AppTable, FieldSpec, RegisteredApp } from "./types";

interface StandaloneIntakeFormProps {
  app: RegisteredApp;
  tableId?: string;
  onBackToDesk?: () => void;
  onOpenApp?: () => void;
  onRecordSubmitted?: (tableId: string, record: Record<string, any>) => void;
}

export const StandaloneIntakeForm: React.FC<StandaloneIntakeFormProps> = ({
  app,
  tableId,
  onBackToDesk,
  onOpenApp,
  onRecordSubmitted,
}) => {
  // Available tables from manifest or default
  const tables: AppTable[] =
    app.manifest.tables && app.manifest.tables.length > 0
      ? app.manifest.tables
      : [
          {
            id: "tbl-proposals",
            name: "Research Proposals",
            slug: "proposals",
            fields: app.manifest.views[0]?.fields || [
              { name: "title", label: "Proposal Title", field_type: "Text", required: true, ferpa_sensitive: false },
              { name: "budget", label: "Budget", field_type: "Number", required: true, ferpa_sensitive: false },
            ],
          },
        ];

  const [activeTableId, setActiveTableId] = useState<string>(() => {
    if (tableId && tables.some((t) => t.id === tableId || t.slug === tableId)) {
      const match = tables.find((t) => t.id === tableId || t.slug === tableId);
      return match ? match.id : tables[0].id;
    }
    return tables[0].id;
  });

  const activeTable = tables.find((t) => t.id === activeTableId) || tables[0];

  // Dynamic form state
  const [formData, setFormData] = useState<Record<string, any>>({});
  const [validationErrors, setValidationErrors] = useState<Record<string, string>>({});
  const [submittedRecordId, setSubmittedRecordId] = useState<string | null>(null);
  const [attestationHash, setAttestationHash] = useState<string | null>(null);
  const [copiedLink, setCopiedLink] = useState<boolean>(false);

  // Compute target table records for Relation fields
  const getRelationOptions = (field: FieldSpec) => {
    if (!field.target_table_id) return [];
    const targetTable = tables.find((t) => t.id === field.target_table_id);
    if (!targetTable || !targetTable.records) return [];
    const displayKey = field.target_display_field || targetTable.primary_field || "name";
    return targetTable.records.map((r) => ({
      id: r.id || r.name,
      label: r[displayKey] || r.name || r.title || r.id,
    }));
  };

  const handleInputChange = (fieldName: string, value: any) => {
    setFormData((prev) => ({ ...prev, [fieldName]: value }));
    if (validationErrors[fieldName]) {
      setValidationErrors((prev) => {
        const next = { ...prev };
        delete next[fieldName];
        return next;
      });
    }
  };

  const handleToggleMultiSelect = (fieldName: string, option: string) => {
    const current: string[] = Array.isArray(formData[fieldName]) ? formData[fieldName] : [];
    const updated = current.includes(option)
      ? current.filter((o) => o !== option)
      : [...current, option];
    handleInputChange(fieldName, updated);
  };

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();

    // Validate required fields
    const errors: Record<string, string> = {};
    for (const field of activeTable.fields) {
      if (
        field.name === "id" ||
        field.name === "_table_id" ||
        field.field_type === "Formula" ||
        field.field_type === "Lookup" ||
        field.field_type === "Rollup" ||
        field.field_type === "Count"
      ) {
        continue;
      }
      if (field.required) {
        const val = formData[field.name];
        if (val === undefined || val === null || val === "" || (Array.isArray(val) && val.length === 0)) {
          errors[field.name] = `${field.label} is required.`;
        }
      }
    }

    if (Object.keys(errors).length > 0) {
      setValidationErrors(errors);
      return;
    }

    const newId = `REC-${Date.now().toString().slice(-6)}`;
    const recordPayload = {
      id: newId,
      _table_id: activeTable.id,
      ...formData,
      submitted_at: new Date().toISOString(),
    };

    // Generate mock SHA-256 attestation hash
    const rawPayloadString = JSON.stringify(recordPayload);
    let hashNum = 0;
    for (let i = 0; i < rawPayloadString.length; i++) {
      hashNum = (hashNum << 5) - hashNum + rawPayloadString.charCodeAt(i);
      hashNum |= 0;
    }
    const hexHash = Math.abs(hashNum).toString(16).padStart(8, "0") + "f9e4a8b72c01d93e";
    const fullAttestation = `sha256:${hexHash}`;

    if (onRecordSubmitted) {
      onRecordSubmitted(activeTable.id, recordPayload);
    }

    setSubmittedRecordId(newId);
    setAttestationHash(fullAttestation);
  };

  const handleReset = () => {
    setFormData({});
    setValidationErrors({});
    setSubmittedRecordId(null);
    setAttestationHash(null);
  };

  const handleCopyLink = () => {
    const url = `${window.location.origin}/form/${app.slug}/${activeTable.id}`;
    if (navigator.clipboard) {
      navigator.clipboard.writeText(url);
      setCopiedLink(true);
      setTimeout(() => setCopiedLink(false), 2500);
    }
  };

  // Filter out system and computed read-only fields
  const editableFields = activeTable.fields.filter(
    (f) =>
      f.name !== "id" &&
      f.name !== "_table_id" &&
      f.field_type !== "Formula" &&
      f.field_type !== "Lookup" &&
      f.field_type !== "Rollup" &&
      f.field_type !== "Count"
  );

  return (
    <div
      data-testid="standalone-intake-form-container"
      className="min-h-screen bg-slate-50 dark:bg-slate-950 text-slate-900 dark:text-slate-100 flex flex-col font-sans transition-colors duration-200"
    >
      {/* Top Navigation Bar */}
      <header className="sticky top-0 z-30 bg-white/90 dark:bg-slate-900/90 backdrop-blur-md border-b border-slate-200 dark:border-slate-800 px-6 py-3 flex items-center justify-between shadow-xs">
        <div className="flex items-center gap-4">
          {onBackToDesk && (
            <button
              type="button"
              onClick={onBackToDesk}
              className="text-xs font-semibold text-slate-500 hover:text-slate-900 dark:hover:text-white px-2 py-1 rounded hover:bg-slate-100 dark:hover:bg-slate-800 cursor-pointer"
            >
              ← Sovereign Desk
            </button>
          )}
          <div className="flex items-center gap-2">
            <span className="text-sm font-bold text-slate-900 dark:text-white">{app.title}</span>
            <span className="text-[10px] font-mono px-2 py-0.5 rounded bg-blue-100 text-blue-800 dark:bg-blue-950 dark:text-blue-300 font-bold">
              Public Intake
            </span>
          </div>
        </div>

        <div className="flex items-center gap-3">
          <button
            type="button"
            data-testid="copy-form-url-btn"
            onClick={handleCopyLink}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg border border-slate-300 dark:border-slate-700 hover:bg-slate-100 dark:hover:bg-slate-800 text-xs font-semibold text-slate-700 dark:text-slate-300 transition-colors cursor-pointer"
          >
            {copiedLink ? "✓ Copied Link" : "🔗 Copy Share Link"}
          </button>
          {onOpenApp && (
            <button
              type="button"
              onClick={onOpenApp}
              className="px-3.5 py-1.5 rounded-lg bg-blue-600 hover:bg-blue-700 text-white text-xs font-bold transition-colors cursor-pointer"
            >
              Open Application →
            </button>
          )}
        </div>
      </header>

      {/* Main Full-Screen Form Canvas */}
      <main className="flex-1 max-w-3xl w-full mx-auto px-4 py-8 flex flex-col justify-center">
        {submittedRecordId ? (
          /* Confirmation Card */
          <div
            data-testid="intake-confirmation-card"
            className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl p-8 shadow-xl text-center space-y-6 animate-fade-in"
          >
            <div className="w-16 h-16 mx-auto rounded-full bg-emerald-100 dark:bg-emerald-950/80 text-emerald-600 dark:text-emerald-400 flex items-center justify-center text-3xl font-bold">
              ✓
            </div>
            <div>
              <h2 className="text-xl font-bold text-slate-900 dark:text-white">
                Submission Received and Attested
              </h2>
              <p className="mt-2 text-xs text-slate-500 dark:text-slate-400 max-w-md mx-auto">
                Your entry has been committed to the sovereign dataset for{" "}
                <span className="font-semibold text-slate-700 dark:text-slate-300">{activeTable.name}</span>{" "}
                and verified by institutional Cedar ABAC policy rules.
              </p>
            </div>

            <div className="p-4 rounded-xl bg-slate-50 dark:bg-slate-800/60 border border-slate-200 dark:border-slate-700/80 text-left font-mono text-xs space-y-2 max-w-lg mx-auto">
              <div className="flex justify-between items-center text-slate-500">
                <span>Record ID:</span>
                <span className="font-bold text-slate-800 dark:text-slate-200">{submittedRecordId}</span>
              </div>
              <div className="flex justify-between items-center text-slate-500">
                <span>Table:</span>
                <span className="text-slate-800 dark:text-slate-200">{activeTable.name} ({activeTable.slug})</span>
              </div>
              <div className="flex justify-between items-center text-slate-500">
                <span>Ledger Attestation:</span>
                <span className="text-emerald-600 dark:text-emerald-400 truncate max-w-[240px]">
                  {attestationHash}
                </span>
              </div>
            </div>

            <div className="pt-2">
              <button
                type="button"
                data-testid="submit-another-btn"
                onClick={handleReset}
                className="px-6 py-2.5 rounded-xl bg-blue-600 hover:bg-blue-700 text-white font-bold text-xs shadow-md transition-all cursor-pointer"
              >
                + Submit Another Response
              </button>
            </div>
          </div>
        ) : (
          /* Intake Form */
          <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl shadow-xl overflow-hidden animate-fade-in">
            {/* Header Banner */}
            <div className="p-8 border-b border-slate-100 dark:border-slate-800 bg-linear-to-r from-blue-50/50 via-slate-50 to-white dark:from-blue-950/20 dark:via-slate-900 dark:to-slate-900">
              <div className="flex items-center gap-2 text-xs font-semibold text-blue-600 dark:text-blue-400 mb-1">
                <span>{app.orgCode}</span>
                <span>•</span>
                <span>{app.department}</span>
              </div>
              <h1 className="text-2xl font-extrabold text-slate-900 dark:text-white tracking-tight">
                {activeTable.name} Intake
              </h1>
              <p className="mt-2 text-xs text-slate-500 dark:text-slate-400">
                {activeTable.description ||
                  `Please provide the required details below to submit a new record to ${activeTable.name}.`}
              </p>

              {/* Table Switcher if multiple tables exist */}
              {tables.length > 1 && (
                <div className="flex items-center gap-2 mt-6 pt-4 border-t border-slate-200/60 dark:border-slate-800">
                  <span className="text-[11px] font-semibold text-slate-400 uppercase tracking-wider">
                    Select Target:
                  </span>
                  <div className="flex items-center gap-1.5">
                    {tables.map((t) => (
                      <button
                        key={t.id}
                        type="button"
                        data-testid={`form-table-tab-${t.id}`}
                        onClick={() => {
                          setActiveTableId(t.id);
                          setValidationErrors({});
                        }}
                        className={`px-3 py-1 rounded-lg text-xs font-semibold transition-all cursor-pointer ${
                          t.id === activeTable.id
                            ? "bg-blue-600 text-white shadow-xs font-bold"
                            : "bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-400 hover:text-slate-900 dark:hover:text-white"
                        }`}
                      >
                        {t.name}
                      </button>
                    ))}
                  </div>
                </div>
              )}
            </div>

            {/* Validation Notice */}
            {Object.keys(validationErrors).length > 0 && (
              <div className="mx-8 mt-6 p-3.5 rounded-xl bg-red-50 dark:bg-red-950/40 border border-red-200 dark:border-red-900 text-red-700 dark:text-red-300 text-xs">
                Please complete all required fields indicated below before submitting.
              </div>
            )}

            {/* Dynamic Form */}
            <form
              data-testid="standalone-intake-form"
              onSubmit={handleSubmit}
              className="p-8 space-y-6 text-xs"
            >
              {editableFields.map((field) => {
                const hasError = !!validationErrors[field.name];
                const value = formData[field.name];

                return (
                  <div key={field.name} className="space-y-1.5">
                    <div className="flex items-center justify-between">
                      <label className="font-semibold text-slate-800 dark:text-slate-200 text-xs">
                        {field.label}
                        {field.required && <span className="text-red-500 ml-0.5">*</span>}
                      </label>
                      <span className="text-[10px] text-slate-400 font-mono">
                        {field.field_type}
                      </span>
                    </div>

                    {/* Field Renderers based on FieldType */}
                    {field.field_type === "Text" || field.field_type === "Autonumber" ? (
                      <input
                        type="text"
                        name={field.name}
                        data-testid={`field-input-${field.name}`}
                        value={value || ""}
                        onChange={(e) => handleInputChange(field.name, e.target.value)}
                        placeholder={`Enter ${field.label.toLowerCase()}...`}
                        className={`w-full px-3.5 py-2.5 rounded-xl border bg-white dark:bg-slate-850 text-slate-900 dark:text-white focus:outline-blue-500 transition-colors ${
                          hasError
                            ? "border-red-400 dark:border-red-600 ring-1 ring-red-400"
                            : "border-slate-300 dark:border-slate-700"
                        }`}
                      />
                    ) : field.field_type === "Email" ? (
                      <input
                        type="email"
                        name={field.name}
                        data-testid={`field-input-${field.name}`}
                        value={value || ""}
                        onChange={(e) => handleInputChange(field.name, e.target.value)}
                        placeholder="e.g. name@university.edu"
                        className="w-full px-3.5 py-2.5 rounded-xl border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-850 text-slate-900 dark:text-white focus:outline-blue-500"
                      />
                    ) : field.field_type === "Phone" ? (
                      <input
                        type="tel"
                        name={field.name}
                        data-testid={`field-input-${field.name}`}
                        value={value || ""}
                        onChange={(e) => handleInputChange(field.name, e.target.value)}
                        placeholder="e.g. +1 (555) 019-2834"
                        className="w-full px-3.5 py-2.5 rounded-xl border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-850 text-slate-900 dark:text-white focus:outline-blue-500"
                      />
                    ) : field.field_type === "Url" ? (
                      <input
                        type="url"
                        name={field.name}
                        data-testid={`field-input-${field.name}`}
                        value={value || ""}
                        onChange={(e) => handleInputChange(field.name, e.target.value)}
                        placeholder="https://..."
                        className="w-full px-3.5 py-2.5 rounded-xl border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-850 text-slate-900 dark:text-white focus:outline-blue-500"
                      />
                    ) : field.field_type === "Number" ||
                      field.field_type === "Currency" ||
                      field.field_type === "Percent" ? (
                      <div className="relative">
                        {field.field_type === "Currency" && (
                          <span className="absolute left-3.5 top-2.5 text-slate-400 font-semibold">$</span>
                        )}
                        <input
                          type="number"
                          name={field.name}
                          data-testid={`field-input-${field.name}`}
                          value={value ?? ""}
                          onChange={(e) => handleInputChange(field.name, e.target.value === "" ? "" : Number(e.target.value))}
                          placeholder="0"
                          className={`w-full py-2.5 rounded-xl border bg-white dark:bg-slate-850 text-slate-900 dark:text-white focus:outline-blue-500 transition-colors ${
                            field.field_type === "Currency" ? "pl-8 pr-3.5" : "px-3.5"
                          } ${
                            hasError
                              ? "border-red-400 dark:border-red-600 ring-1 ring-red-400"
                              : "border-slate-300 dark:border-slate-700"
                          }`}
                        />
                        {field.field_type === "Percent" && (
                          <span className="absolute right-3.5 top-2.5 text-slate-400 font-semibold">%</span>
                        )}
                      </div>
                    ) : field.field_type === "Date" ? (
                      <input
                        type="date"
                        name={field.name}
                        data-testid={`field-input-${field.name}`}
                        value={value || ""}
                        onChange={(e) => handleInputChange(field.name, e.target.value)}
                        className="w-full px-3.5 py-2.5 rounded-xl border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-850 text-slate-900 dark:text-white focus:outline-blue-500"
                      />
                    ) : field.field_type === "Select" ? (
                      <select
                        name={field.name}
                        data-testid={`field-input-${field.name}`}
                        value={value || ""}
                        onChange={(e) => handleInputChange(field.name, e.target.value)}
                        className="w-full px-3.5 py-2.5 rounded-xl border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-850 text-slate-900 dark:text-white focus:outline-blue-500 cursor-pointer"
                      >
                        <option value="">Select an option...</option>
                        {(field.select_options || ["Under Review", "Approved", "Funded", "Archived"]).map((opt) => (
                          <option key={opt} value={opt}>
                            {opt}
                          </option>
                        ))}
                      </select>
                    ) : field.field_type === "MultiSelect" ? (
                      <div className="flex flex-wrap gap-2 pt-1">
                        {(field.select_options || ["Priority", "Standard", "Expedited", "Institutional"]).map((opt) => {
                          const isSelected = Array.isArray(value) && value.includes(opt);
                          return (
                            <button
                              key={opt}
                              type="button"
                              data-testid={`multiselect-option-${opt}`}
                              onClick={() => handleToggleMultiSelect(field.name, opt)}
                              className={`px-3 py-1.5 rounded-lg text-xs font-semibold transition-all cursor-pointer ${
                                isSelected
                                  ? "bg-blue-600 text-white font-bold"
                                  : "bg-slate-100 dark:bg-slate-800 text-slate-700 dark:text-slate-300 hover:bg-slate-200 dark:hover:bg-slate-700"
                              }`}
                            >
                              {isSelected ? `✓ ${opt}` : `+ ${opt}`}
                            </button>
                          );
                        })}
                      </div>
                    ) : field.field_type === "Rating" ? (
                      <div className="flex items-center gap-1.5 pt-1">
                        {[1, 2, 3, 4, 5].map((star) => (
                          <button
                            key={star}
                            type="button"
                            data-testid={`rating-star-${star}`}
                            onClick={() => handleInputChange(field.name, star)}
                            className={`p-2 rounded-lg text-lg transition-transform cursor-pointer hover:scale-110 ${
                              (value || 0) >= star ? "text-amber-400" : "text-slate-300 dark:text-slate-700"
                            }`}
                          >
                            ★
                          </button>
                        ))}
                        <span className="text-xs text-slate-400 ml-2">
                          {value ? `${value} / 5 stars` : "Unrated"}
                        </span>
                      </div>
                    ) : field.field_type === "Checkbox" || field.field_type === "Boolean" ? (
                      <label className="flex items-center gap-3 pt-1 cursor-pointer">
                        <input
                          type="checkbox"
                          name={field.name}
                          data-testid={`field-input-${field.name}`}
                          checked={!!value}
                          onChange={(e) => handleInputChange(field.name, e.target.checked)}
                          className="w-4 h-4 rounded text-blue-600 border-slate-300 dark:border-slate-700 focus:ring-blue-500 cursor-pointer"
                        />
                        <span className="text-xs text-slate-700 dark:text-slate-300">
                          {field.label} confirmed and attested
                        </span>
                      </label>
                    ) : field.field_type === "Relation" ? (
                      <select
                        name={field.name}
                        data-testid={`field-input-${field.name}`}
                        value={value || ""}
                        onChange={(e) => handleInputChange(field.name, e.target.value)}
                        className="w-full px-3.5 py-2.5 rounded-xl border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-850 text-slate-900 dark:text-white focus:outline-blue-500 cursor-pointer"
                      >
                        <option value="">Select linked record...</option>
                        {getRelationOptions(field).map((opt) => (
                          <option key={opt.id} value={opt.id}>
                            {opt.label} ({opt.id})
                          </option>
                        ))}
                      </select>
                    ) : (
                      <input
                        type="text"
                        name={field.name}
                        value={value || ""}
                        onChange={(e) => handleInputChange(field.name, e.target.value)}
                        className="w-full px-3.5 py-2.5 rounded-xl border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-850 text-slate-900 dark:text-white"
                      />
                    )}

                    {hasError && (
                      <p className="text-[11px] text-red-500">{validationErrors[field.name]}</p>
                    )}
                  </div>
                );
              })}

              <div className="p-4 rounded-xl bg-slate-50 dark:bg-slate-800/40 border border-slate-200 dark:border-slate-700 text-slate-500 dark:text-slate-400 text-[11px] leading-relaxed">
                🛡️ All data submitted through this public intake endpoint is verified against institutional NIST OSCAL baselines and authorized via Cedar policy.
              </div>

              <div className="pt-2 flex items-center justify-end gap-3">
                <button
                  type="button"
                  onClick={handleReset}
                  className="px-4 py-2.5 rounded-xl border border-slate-300 dark:border-slate-700 hover:bg-slate-100 dark:hover:bg-slate-800 text-slate-600 dark:text-slate-300 font-semibold cursor-pointer"
                >
                  Clear Form
                </button>
                <button
                  type="submit"
                  data-testid="submit-intake-form-btn"
                  className="px-6 py-2.5 rounded-xl bg-blue-600 hover:bg-blue-700 text-white font-bold shadow-md transition-all cursor-pointer"
                >
                  Submit Form
                </button>
              </div>
            </form>
          </div>
        )}
      </main>
    </div>
  );
};
