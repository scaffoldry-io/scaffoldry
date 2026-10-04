import React from "react";
import { createRoot } from "react-dom/client";
import { ManifestRenderer } from "./ManifestRenderer";
import { AppManifest } from "./types";

const sampleManifest: AppManifest = {
  slug: "bio-lab-inventory",
  title: "Biology Lab Equipment Inventory",
  description: "Dynamic departmental instrument register and CEDS crosswalk",
  organization_code: "DEPT-BIO",
  department: "biology",
  herm_capability_id: "2.2.3",
  custom_domain: "bio-inventory.science.state.edu",
  custom_domain_verified: true,
  views: [
    {
      id: "lab-form",
      title: "Instrument Registration",
      view_type: "Form",
      fields: [
        {
          name: "item_name",
          label: "Equipment Name",
          field_type: "Text",
          required: true,
          ferpa_sensitive: false,
        },
        {
          name: "serial_number",
          label: "Serial Number",
          field_type: "Text",
          required: true,
          ferpa_sensitive: false,
        },
        {
          name: "operator_eval",
          label: "Student Operator Evaluation",
          field_type: "Text",
          required: false,
          ferpa_sensitive: true,
        },
      ],
    },
  ],
  ceds_mappings: {
    item_name: "000185",
  },
};

export function App() {
  return (
    <main>
      <ManifestRenderer
        manifest={sampleManifest}
        onSubmitRecord={(record) => {
          console.log("Submitted dynamic record:", record);
        }}
      />
    </main>
  );
}

const rootEl = document.getElementById("root");
if (rootEl) {
  createRoot(rootEl).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>
  );
}
