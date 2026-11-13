import React from "react";
import { createRoot } from "react-dom/client";
import { AdminDesk } from "./AdminDesk";
import { OAuthConsent } from "./OAuthConsent";
import { SharedView } from "./SharedView";
import "./index.css";

export function App() {
  // An outside app sends the person here to sign in. The page reads the request from its address.
  if (typeof window !== "undefined" && window.location.pathname === "/oauth/authorize") {
    return <OAuthConsent />;
  }
  // A shared link: `/s/<token>`. It opens one view, read only.
  const shared = typeof window !== "undefined" ? window.location.pathname.match(/^\/s\/([A-Za-z0-9]{16,64})$/) : null;
  if (shared) {
    return <SharedView token={shared[1]} />;
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
