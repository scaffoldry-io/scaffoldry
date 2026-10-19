// Fixtures made through MCP tools: an app, some rows, an edit, a stale edit, and a delete. Every step is checked, so
// this is also the write test for the endpoint. The data is what the UI audit and the UI smoke flow then look at.
// Returns { results, fixture }. results is a list of { name, ok, detail }.
export async function seedFixtures(client) {
  const results = [];
  const check = (name, ok, detail = "") => results.push({ name, ok: !!ok, detail: ok ? "" : detail });

  const workspaces = (await client.call("list_workspaces")).value ?? [];
  const workspace = [...workspaces].sort((a, b) => a.id.localeCompare(b.id))[0];
  check("a workspace exists to put the app in", workspace, "list_workspaces returned none");
  if (!workspace) return { results, fixture: null };

  const slug = `fx-${Math.random().toString(36).slice(2, 8)}`;
  const name = { name: "name", label: "Name", field_type: "Text", required: true, ferpa_sensitive: false };
  const amount = { name: "amount", label: "Amount", field_type: "Number", required: false, ferpa_sensitive: false };
  const manifest = {
    slug, title: "Fixture Register", description: "Made through MCP tools", organization_code: "FX", department: workspace.department,
    workspace_id: workspace.id, custom_domain_verified: false,
    tables: [{ id: "t", name: "Items", slug: "items", fields: [name, amount], primary_field: "name", sample_records: [] }], relationships: [],
    views: [{ id: "v", table_id: "t", title: "All", view_type: "Grid", fields: [name, amount] }], ceds_mappings: {}, pages: [], access: {},
  };

  const proposed = await client.call("propose_app_change", { workspace_id: workspace.id, summary: "Create the Fixture Register", manifest });
  check("propose_app_change holds a new app as a proposal", proposed.ok && proposed.value?.status === "Pending", proposed.message.slice(0, 120));
  const failed = proposed.value?.checks?.filter((c) => c.outcome !== "pass") ?? [];
  check("the proposal's checks all pass", failed.length === 0, failed.map((c) => c.name).join(","));
  const decided = await client.call("decide_proposal", { proposal_id: proposed.value?.proposal_id, decision: "approve", rationale: "Test fixture" });
  check("decide_proposal approves it", decided.ok && decided.value?.status === "Approved", decided.message.slice(0, 120));

  const apps = (await client.call("list_apps", { workspace_id: workspace.id })).value ?? [];
  check("the app is now listed", apps.some((a) => a.slug === slug));
  const described = await client.call("describe_app", { app_slug: slug });
  check("describe_app returns its tables", described.ok && JSON.stringify(described.value).includes("Items"), described.message.slice(0, 120));

  const rows = [["Fixture row 1", 10], ["Fixture row 2", 25], ["Café Ünïcode row", 40], ["Fixture row 4", 55], ["Fixture row 5", 70]];
  const created = [];
  for (const [n, a] of rows) {
    const r = await client.call("create_record", { app_slug: slug, data: { _table_id: "t", name: n, amount: a } });
    created.push(r);
  }
  check("create_record adds each row", created.every((r) => r.ok && r.value?.id), created.find((r) => !r.ok)?.message.slice(0, 120));
  const first = created[0]?.value;
  check("a new row starts at version 1", first?.version === 1, `version ${first?.version}`);

  const edited = await client.call("update_record", { app_slug: slug, record_id: first?.id, version: first?.version, data: { amount: 99 } });
  check("update_record with the version read saves, and moves the version on", edited.ok && edited.value?.version === 2, edited.message.slice(0, 120));
  check("a field not named keeps its value", edited.value?.data?.name === "Fixture row 1", JSON.stringify(edited.value?.data));

  const stale = await client.call("update_record", { app_slug: slug, record_id: first?.id, version: 1, data: { amount: 1 } });
  check("a stale version is an error that names the current one", !stale.ok && /current version is 2\b/.test(stale.message), stale.message.slice(0, 160));

  const victim = created[4]?.value;
  const deleted = await client.call("delete_record", { app_slug: slug, record_id: victim?.id, version: victim?.version });
  check("delete_record moves a row to the trash", deleted.ok, deleted.message.slice(0, 120));

  const listed = (await client.call("list_records", { app_slug: slug, table_id: "t" })).value;
  const records = listed?.records ?? listed?.items ?? [];
  const names = records.map((r) => r.data?.name);
  check("list_records shows four rows, with the edit and without the deleted one", records.length === 4 && !names.includes("Fixture row 5") && records.find((r) => r.id === first?.id)?.data?.amount === 99, `names ${JSON.stringify(names)}`);

  return { results, fixture: { workspace: workspace.id, app: slug, title: "Fixture Register", rowNames: names } };
}
