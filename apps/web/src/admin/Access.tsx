import { useEffect, useState } from "react";
import { apiClient } from "../api";
import { PageHeader, Tabs } from "../ui";
import { AccessHierarchies } from "./AccessHierarchies";
import { AccessGrants } from "./AccessGrants";
import { AccessObjects } from "./AccessObjects";
import { AccessPeople } from "./AccessPeople";
import { AccessReviews } from "./AccessReviews";
import { AccessReceipts } from "./AccessReceipts";

type TabId = "hierarchies" | "grants" | "objects" | "people" | "reviews" | "receipts";

export interface AccessViewer {
  eppn: string;
  /** A Platform Admin or compliance. An Org Unit Admin is not: they see the grants inside their units. */
  platform: boolean;
}

// The Access section of the console: who can reach what, and how that changes.
export function Access() {
  const [viewer, setViewer] = useState<AccessViewer | null>(null);
  const [tab, setTab] = useState<TabId | null>(null);

  useEffect(() => {
    let live = true;
    apiClient
      .getCurrentUser()
      .then((me) => {
        if (!live) return;
        const platform = Boolean(me.is_platform_admin || me.user?.affiliation === "compliance" || me.user?.affiliation === "central_admin");
        setViewer({ eppn: String(me.user?.eppn ?? ""), platform });
        setTab((t) => t ?? (platform ? "hierarchies" : "grants"));
      })
      .catch(() => {
        if (!live) return;
        setViewer({ eppn: "", platform: false });
        setTab((t) => t ?? "grants");
      });
    return () => {
      live = false;
    };
  }, []);

  if (!viewer || !tab) return <div data-testid="admin-access" aria-busy="true" />;

  const tabs: { id: TabId; label: string }[] = [
    ...(viewer.platform ? [{ id: "hierarchies" as const, label: "Hierarchies" }] : []),
    { id: "grants", label: "Grants" },
    { id: "objects", label: "Objects" },
    { id: "people", label: "People" },
    { id: "reviews", label: "Reviews" },
    ...(viewer.platform ? [{ id: "receipts" as const, label: "Receipts" }] : []),
  ];

  return (
    <div className="space-y-6" data-testid="admin-access">
      <PageHeader title="Access" description="Who can reach what, why, and how that changes" />
      <Tabs ariaLabel="Access" selectedId={tab} onSelect={(id) => setTab(id as TabId)} tabs={tabs} />
      {tab === "hierarchies" && <AccessHierarchies viewer={viewer} />}
      {tab === "grants" && <AccessGrants viewer={viewer} />}
      {tab === "objects" && <AccessObjects />}
      {tab === "people" && <AccessPeople />}
      {tab === "reviews" && <AccessReviews />}
      {tab === "receipts" && <AccessReceipts />}
    </div>
  );
}
