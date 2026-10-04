import React from "react";
import { createRoot } from "react-dom/client";
import { AdminDesk } from "./AdminDesk";
import "./index.css";

export function App() {
  return <AdminDesk />;
}

const rootEl = document.getElementById("root");
if (rootEl) {
  createRoot(rootEl).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>
  );
}
