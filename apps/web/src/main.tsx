import React from "react";
import { createRoot } from "react-dom/client";
import { AdminDesk } from "./AdminDesk";
import { OAuthConsent } from "./OAuthConsent";
import "./index.css";

export function App() {
  // An outside app sends the person here to sign in. The page reads the request from its address.
  if (typeof window !== "undefined" && window.location.pathname === "/oauth/authorize") {
    return <OAuthConsent />;
  }
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
