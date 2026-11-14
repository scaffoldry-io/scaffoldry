import React from "react";
import { createRoot } from "react-dom/client";
import { AdminDesk } from "./AdminDesk";
import { OAuthConsent } from "./OAuthConsent";
import { GuestPage, type GuestRoute } from "./GuestPage";
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
  // A guest's pages: `/g/<link>` trades a link for a session, `/g/stop/<link>` stops mail, `/g/<app>/<form>` is the status page.
  const path = typeof window !== "undefined" ? window.location.pathname : "";
  const guest = guestRoute(path);
  if (guest) {
    return <GuestPage route={guest} />;
  }
  return <AdminDesk />;
}

export function guestRoute(path: string): GuestRoute | null {
  const stop = path.match(/^\/g\/stop\/([A-Za-z0-9]{16,64})$/);
  if (stop) return { kind: "stop", token: stop[1] };
  const link = path.match(/^\/g\/([A-Za-z0-9]{16,64})$/);
  if (link) return { kind: "link", token: link[1] };
  const page = path.match(/^\/g\/([A-Za-z0-9._-]{1,64})\/([0-9a-f-]{36})$/);
  return page ? { kind: "page", app: page[1], form: page[2] } : null;
}

const rootEl = document.getElementById("root");
if (rootEl) {
  createRoot(rootEl).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>
  );
}
