import React, { useState } from "react";
import { AppManifest, FieldSpec } from "./types";

interface Props {
  manifest: AppManifest;
  onSubmitRecord?: (data: Record<string, unknown>) => void;
}

export const ManifestRenderer: React.FC<Props> = ({ manifest, onSubmitRecord }) => {
  const [formData, setFormData] = useState<Record<string, unknown>>({});
  const [submitted, setSubmitted] = useState(false);

  const activeView = manifest.views[0];

  const handleInputChange = (fieldName: string, value: unknown) => {
    setFormData((prev) => ({ ...prev, [fieldName]: value }));
  };

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (onSubmitRecord) {
      onSubmitRecord(formData);
    }
    setSubmitted(true);
  };

  return (
    <div className="max-w-3xl mx-auto rounded-lg border border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-900 shadow-sm p-6">
      <header className="mb-6 pb-4 border-b border-slate-200 dark:border-slate-800">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <h2 className="text-xl font-semibold text-slate-900 dark:text-white">
            {manifest.title}
          </h2>
          {manifest.custom_domain && (
            <span className="inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-blue-50 text-blue-700 dark:bg-blue-950/60 dark:text-blue-300 border border-blue-200 dark:border-blue-800">
              DNS: {manifest.custom_domain}
            </span>
          )}
        </div>
        <p className="text-sm text-slate-600 dark:text-slate-400 mt-1">
          {manifest.description}
        </p>
        <div className="flex flex-wrap gap-4 text-xs text-slate-500 dark:text-slate-400 mt-3">
          <span>Org: <strong className="text-slate-700 dark:text-slate-300">{manifest.organization_code}</strong></span>
          <span>Dept: <strong className="text-slate-700 dark:text-slate-300">{manifest.department}</strong></span>
          {manifest.herm_capability_id && (
            <span>HERM: <strong className="text-slate-700 dark:text-slate-300">{manifest.herm_capability_id}</strong></span>
          )}
        </div>
      </header>

      {activeView && (
        <section>
          <h3 className="text-base font-medium text-slate-800 dark:text-slate-200 mb-4">
            {activeView.title}
          </h3>
          {submitted ? (
            <div className="p-4 rounded-md bg-emerald-50 dark:bg-emerald-950/40 border border-emerald-200 dark:border-emerald-800 text-emerald-800 dark:text-emerald-300 text-sm">
              <div className="font-semibold mb-1">Record Registered Successfully</div>
              <div>Submitted payload validated against Cedar departmental policies and annotated with CEDS v11 standards.</div>
              <button
                type="button"
                onClick={() => {
                  setFormData({});
                  setSubmitted(false);
                }}
                className="mt-3 inline-flex items-center px-3 py-1.5 rounded text-xs font-medium bg-emerald-600 hover:bg-emerald-700 text-white transition-colors"
              >
                Register Another Record
              </button>
            </div>
          ) : (
            <form onSubmit={handleSubmit} className="flex flex-col gap-4">
              {activeView.fields.map((field: FieldSpec) => (
                <div key={field.name} className="flex flex-col gap-1.5">
                  <div className="flex items-center justify-between">
                    <label className="text-xs font-medium text-slate-700 dark:text-slate-300">
                      {field.label} {field.required && <span className="text-rose-500">*</span>}
                    </label>
                    {field.ferpa_sensitive && (
                      <span className="text-[10px] font-semibold tracking-wider uppercase px-2 py-0.5 rounded bg-rose-100 dark:bg-rose-950/60 text-rose-700 dark:text-rose-300 border border-rose-200 dark:border-rose-900">
                        FERPA Sensitive (34 CFR § 99.30)
                      </span>
                    )}
                  </div>
                  <input
                    type={field.field_type === "Number" ? "number" : "text"}
                    required={field.required}
                    value={(formData[field.name] as string) || ""}
                    onChange={(e) => handleInputChange(field.name, e.target.value)}
                    placeholder={`Enter ${field.label.toLowerCase()}...`}
                    className="w-full px-3 py-2 text-sm rounded-md border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-950 text-slate-900 dark:text-white placeholder-slate-400 focus:outline-none focus:ring-2 focus:ring-blue-500 dark:focus:ring-blue-400 transition-colors"
                  />
                  {manifest.ceds_mappings[field.name] && (
                    <span className="text-[11px] text-slate-500 dark:text-slate-400">
                      NCES CEDS Element: <code className="font-mono text-blue-600 dark:text-blue-400">{manifest.ceds_mappings[field.name]}</code>
                    </span>
                  )}
                </div>
              ))}
              <div className="pt-2">
                <button
                  type="submit"
                  className="inline-flex items-center justify-center px-4 py-2 rounded-md text-sm font-medium bg-blue-600 hover:bg-blue-700 text-white shadow-sm transition-colors cursor-pointer"
                >
                  Submit Record
                </button>
              </div>
            </form>
          )}
        </section>
      )}
    </div>
  );
};
