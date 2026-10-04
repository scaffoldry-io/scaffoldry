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
    <div style={{ maxWidth: "800px", margin: "2rem auto", fontFamily: "sans-serif", padding: "1.5rem", borderRadius: "8px", border: "1px solid #e2e8f0" }}>
      <header style={{ marginBottom: "1.5rem", borderBottom: "1px solid #cbd5e1", paddingBottom: "1rem" }}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <h1 style={{ margin: 0, fontSize: "1.5rem" }}>{manifest.title}</h1>
          {manifest.custom_domain && (
            <span style={{ background: "#e0f2fe", color: "#0369a1", padding: "4px 8px", borderRadius: "4px", fontSize: "0.85rem", fontWeight: "bold" }}>
              DNS: {manifest.custom_domain}
            </span>
          )}
        </div>
        <p style={{ color: "#64748b", margin: "0.5rem 0" }}>{manifest.description}</p>
        <div style={{ display: "flex", gap: "1rem", fontSize: "0.8rem", color: "#475569" }}>
          <span>Org: <strong>{manifest.organization_code}</strong></span>
          <span>Dept: <strong>{manifest.department}</strong></span>
          {manifest.herm_capability_id && (
            <span>HERM: <strong>{manifest.herm_capability_id}</strong></span>
          )}
        </div>
      </header>

      {activeView && (
        <section>
          <h2>{activeView.title}</h2>
          {submitted ? (
            <div style={{ padding: "1rem", background: "#f0fdf4", color: "#166534", borderRadius: "4px" }}>
              Record submitted successfully and mapped to CEDS standards.
            </div>
          ) : (
            <form onSubmit={handleSubmit} style={{ display: "flex", flexDirection: "column", gap: "1rem" }}>
              {activeView.fields.map((field: FieldSpec) => (
                <div key={field.name} style={{ display: "flex", flexDirection: "column", gap: "0.25rem" }}>
                  <label style={{ fontWeight: 600, fontSize: "0.9rem" }}>
                    {field.label} {field.required && <span style={{ color: "#ef4444" }}>*</span>}
                    {field.ferpa_sensitive && (
                      <span style={{ marginLeft: "8px", fontSize: "0.75rem", background: "#fee2e2", color: "#991b1b", padding: "2px 6px", borderRadius: "3px" }}>
                        FERPA SENSITIVE
                      </span>
                    )}
                  </label>
                  <input
                    type={field.field_type === "Number" ? "number" : "text"}
                    required={field.required}
                    value={(formData[field.name] as string) || ""}
                    onChange={(e) => handleInputChange(field.name, e.target.value)}
                    style={{ padding: "0.5rem", borderRadius: "4px", border: "1px solid #cbd5e1" }}
                  />
                  {manifest.ceds_mappings[field.name] && (
                    <span style={{ fontSize: "0.75rem", color: "#64748b" }}>
                      Mapped to CEDS Element: {manifest.ceds_mappings[field.name]}
                    </span>
                  )}
                </div>
              ))}
              <button
                type="submit"
                style={{ marginTop: "1rem", padding: "0.6rem 1.2rem", background: "#0284c7", color: "white", border: "none", borderRadius: "4px", fontWeight: "bold", cursor: "pointer" }}
              >
                Submit Record
              </button>
            </form>
          )}
        </section>
      )}
    </div>
  );
};
