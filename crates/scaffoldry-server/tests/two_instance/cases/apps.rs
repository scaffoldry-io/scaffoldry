//! Apps, records, comments, views, history, trash, processes, proposals and jobs.

use super::*;

pub fn add(m: &mut Entries) {
    m.insert("POST /api/v1/workspaces/{id}/apps".into(), case!(app_create));
    m.insert("PUT /api/v1/apps/{slug}".into(), case!(app_update));
    m.insert("POST /api/v1/apps/{slug}/publish".into(), case!(app_publish));
    m.insert("POST /api/v1/apps/{slug}/pages/{id}/call".into(), case!(page_call_route));
    m.insert("POST /api/v1/apps/{slug}/automations".into(), case!(automation_create));
    exempt(m, "POST /api/v1/apps/{slug}/automations/simulate", "runs a rule against a sample and stores nothing");
    exempt(m, "POST /api/v1/apps/{slug}/access/preview", "asks what a person could see and stores nothing");
    exempt(m, "POST /api/v1/apps/{slug}/fields/convert-preview", "reports what a field conversion would do and stores nothing");
    m.insert("POST /api/v1/proposals/{id}/decision".into(), case!(proposal_decide));
    m.insert("POST /api/v1/proposals/{id}/preview".into(), case!(proposal_preview));
    m.insert("POST /api/v1/proposals/{id}/trial".into(), case!(proposal_trial));
    m.insert("POST /api/v1/me/trials/{id}/open".into(), case!(trial_reopen));
    m.insert("POST /api/v1/workspaces/{id}/bundle-import".into(), case!(bundle_import));
    exempt(m, "POST /api/v1/apps/{slug}/bundle", "reads an app's definition into a document and stores nothing");

    m.insert("POST /api/v1/apps/{slug}/records".into(), case!(record_create));
    m.insert("PATCH /api/v1/apps/{slug}/records/{id}".into(), case!(record_patch));
    m.insert("DELETE /api/v1/apps/{slug}/records/{id}".into(), case!(record_delete));
    m.insert("POST /api/v1/apps/{slug}/tables/{table_id}/records".into(), case!(table_record_create));
    m.insert("PATCH /api/v1/apps/{slug}/tables/{table_id}/records/{id}".into(), case!(table_record_patch));
    m.insert("DELETE /api/v1/apps/{slug}/tables/{table_id}/records/{id}".into(), case!(table_record_delete));
    m.insert("POST /api/v1/apps/{slug}/records/{id}/restore".into(), case!(record_restore_version));
    m.insert("POST /api/v1/apps/{slug}/trash/{what}/restore".into(), case!(trash_restore));
    m.insert("POST /api/v1/apps/{slug}/history/{batch_id}/undo".into(), case!(history_undo));

    m.insert("POST /api/v1/apps/{slug}/records/{id}/comments".into(), case!(comment_create));
    m.insert("PATCH /api/v1/apps/{slug}/comments/{id}".into(), case!(comment_edit));
    m.insert("DELETE /api/v1/apps/{slug}/comments/{id}".into(), case!(comment_delete));

    m.insert("POST /api/v1/apps/{slug}/views".into(), case!(view_create));
    m.insert("PUT /api/v1/apps/{slug}/views/{id}".into(), case!(view_update));
    m.insert("DELETE /api/v1/apps/{slug}/views/{id}".into(), case!(view_delete));
    m.insert("POST /api/v1/apps/{slug}/views/{id}/duplicate".into(), case!(view_duplicate));

    m.insert("POST /api/v1/apps/{slug}/processes/{id}/decide".into(), case!(process_decide));
    m.insert("POST /api/v1/apps/{slug}/view-as".into(), case!(view_as_start));
    m.insert("POST /api/v1/auth/stop-view-as".into(), case!(view_as_stop));

    for root in ["/api/v1/admin", "/admin"] {
        let (cancel, retry) = (format!("POST {root}/jobs/{{id}}/cancel"), format!("POST {root}/jobs/{{id}}/retry"));
        m.insert(cancel, if root == "/admin" { case!(admin_job_cancel, "/admin") } else { case!(admin_job_cancel, "/api/v1/admin") });
        m.insert(retry, if root == "/admin" { case!(admin_job_retry, "/admin") } else { case!(admin_job_retry, "/api/v1/admin") });
    }
    m.insert("POST /api/v1/jobs/{id}/cancel".into(), case!(job_cancel));
}

/// A job that is queued, made through the database the two servers share. No runner is started, so it stays queued.
pub(super) fn enqueue_job(w: &World, by: &str) -> Result<String, String> {
    let repo = scaffoldry_server::repository::PostgresRepository::connect(Some(w.db.url())).map_err(|e| e.to_string())?;
    scaffoldry_server::jobs::enqueue(&repo, "purge_jobs", json!({}), by, None).map(|id| id.to_string()).map_err(|e| e.to_string())
}

fn app_rule(id: &str, status: &str) -> Value {
    json!({
        "id": id, "name": format!("Rule {id}"), "enabled": true, "trigger": "RecordCreated",
        "predicates": [{ "field_name": "gpa", "operator": "GreaterThan", "expected_value": "3.85" }],
        "steps": [{ "id": "s1", "when": [], "kind": { "Service": { "action": { "UpdateRecordStatus": { "new_status": status } } } } }]
    })
}

/// B reads one record. `None` when B says it is not there.
pub(super) async fn read_record(w: &World, slug: &str, id: &str) -> Result<Option<Value>, String> {
    let (s, body) = w.rb("GET", &format!("/api/v1/apps/{slug}/records/{id}"), OWNER, None).await;
    match s {
        StatusCode::OK => Ok(Some(body)),
        StatusCode::NOT_FOUND => Ok(None),
        other => Err(format!("B read the record: {other} {body}")),
    }
}

// --- apps ----------------------------------------------------------------------------------------------------------------

async fn app_create(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let seen = status(w.rb("GET", &format!("/api/v1/apps/{slug}/schema"), OWNER, None).await, StatusCode::OK, "B reads the schema")?;
    ensure(has(&seen, "gpa"), format!("B serves the app A created: {seen}"))
}

async fn app_update(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let current = status(w.wa("GET", &format!("/api/v1/apps/{slug}"), OWNER, None).await, StatusCode::OK, "A reads the app")?;
    let version = current["version"].as_i64().ok_or("the app has a version")?;
    let mut changed = manifest(&slug);
    changed["tables"][0]["fields"].as_array_mut().ok_or("fields")?.push(json!({ "name": "nickname", "label": "nickname", "field_type": "Text", "required": false, "ferpa_sensitive": false }));
    changed["version"] = json!(version);
    let (s, body) = w.wa("PUT", &format!("/api/v1/apps/{slug}"), OWNER, Some(changed)).await;
    ensure(s == StatusCode::OK, format!("the owner's save applies at once: {s} {body}"))?;
    let seen = status(w.rb("GET", &format!("/api/v1/apps/{slug}/schema"), OWNER, None).await, StatusCode::OK, "B reads the schema")?;
    ensure(has(&seen, "nickname"), format!("B shows the field A saved: {seen}"))
}

async fn app_publish(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let domain = format!("{}.state.edu", unique("p6dom"));
    let (s, body) = w.wa("POST", &format!("/api/v1/apps/{slug}/publish"), OWNER, Some(json!({ "custom_domain": domain }))).await;
    ensure(s == StatusCode::OK || s == StatusCode::ACCEPTED, format!("publish: {s} {body}"))?;
    let seen = status(w.rb("GET", &format!("/api/v1/apps/{slug}"), OWNER, None).await, StatusCode::OK, "B reads the app")?;
    ensure(has(&seen, &domain), format!("B shows the domain A set: {seen}"))
}

async fn automation_create(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let id = unique("p6rule");
    status(w.wa("POST", &format!("/api/v1/apps/{slug}/automations"), OWNER, Some(app_rule(&id, "Approved"))).await, StatusCode::CREATED, "create the rule")?;
    let seen = status(w.rb("GET", &format!("/api/v1/apps/{slug}/automations"), OWNER, None).await, StatusCode::OK, "B lists rules")?;
    ensure(has(&seen, &id), format!("B lists the rule A made: {seen}"))
}

async fn proposal_decide(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let mut changed = manifest(&slug);
    changed["tables"][0]["fields"].as_array_mut().ok_or("fields")?.push(json!({ "name": "pet", "label": "pet", "field_type": "Text", "required": false, "ferpa_sensitive": false }));
    let proposed = w.tool_a(OWNER, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": changed, "summary": "A field" })).await?;
    let id = proposed["proposal_id"].as_str().ok_or("a proposal id")?.to_string();
    status(w.wa("POST", &format!("/api/v1/proposals/{id}/decision"), OWNER, Some(json!({ "decision": "approve", "rationale": "Reviewed", "digest": proposed["digest"] }))).await, StatusCode::OK, "approve")?;
    let seen = status(w.rb("GET", &format!("/api/v1/apps/{slug}/schema"), OWNER, None).await, StatusCode::OK, "B reads the schema")?;
    ensure(has(&seen, "pet"), format!("B shows the field A approved: {seen}"))
}

/// A opens a preview. B serves its token while the proposal is pending and refuses it once A approves.
async fn proposal_preview(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let mut changed = manifest(&slug);
    changed["tables"][0]["fields"].as_array_mut().ok_or("fields")?.push(json!({ "name": "pet", "label": "pet", "field_type": "Text", "required": false, "ferpa_sensitive": false }));
    let proposed = w.tool_a(OWNER, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": changed, "summary": "A field" })).await?;
    let id = proposed["proposal_id"].as_str().ok_or("a proposal id")?.to_string();
    let opened = status(w.wa("POST", &format!("/api/v1/proposals/{id}/preview"), OWNER, None).await, StatusCode::OK, "A opens the preview")?;
    let (token, shadow) = (opened["token"].as_str().ok_or("a token")?.to_string(), opened["app_slug"].as_str().ok_or("an app slug")?.to_string());
    let seen = status(World::call_with(&w.b, "GET", &format!("/api/v1/apps/{shadow}/schema"), &token, None).await, StatusCode::OK, "B serves the preview token")?;
    ensure(has(&seen, "pet"), format!("B shows the proposed field: {seen}"))?;
    status(w.wa("POST", &format!("/api/v1/proposals/{id}/decision"), OWNER, Some(json!({ "decision": "approve", "rationale": "Reviewed", "digest": proposed["digest"] }))).await, StatusCode::OK, "A approves")?;
    status(World::call_with(&w.b, "GET", &format!("/api/v1/apps/{shadow}/schema"), &token, None).await, StatusCode::UNAUTHORIZED, "B refuses the token once the proposal is decided")?;
    Ok(())
}

/// A proposal for a changed app, pending.
async fn pending_change(w: &World) -> Result<(String, Value), String> {
    let slug = make_app(w).await?;
    let mut changed = manifest(&slug);
    changed["tables"][0]["fields"].as_array_mut().ok_or("fields")?.push(json!({ "name": "pet", "label": "pet", "field_type": "Text", "required": false, "ferpa_sensitive": false }));
    let proposed = w.tool_a(OWNER, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": changed, "summary": "A field" })).await?;
    Ok((proposed["proposal_id"].as_str().ok_or("a proposal id")?.to_string(), proposed))
}

/// A makes a trial copy. B reads its synthetic records, and neither finds it once A approves the proposal.
async fn proposal_trial(w: World) -> Outcome {
    let (id, proposed) = pending_change(&w).await?;
    let trial = status(w.wa("POST", &format!("/api/v1/proposals/{id}/trial"), OWNER, None).await, StatusCode::CREATED, "A makes the trial")?;
    let slug = trial["app_slug"].as_str().ok_or("a trial slug")?.to_string();
    let seen = status(w.rb("GET", &format!("/api/v1/apps/{slug}/tables/apps/records"), OWNER, None).await, StatusCode::OK, "B reads the trial's records")?;
    ensure(seen["records"].as_array().is_some_and(|r| !r.is_empty()), format!("B finds the synthetic records: {seen}"))?;
    status(w.wa("POST", &format!("/api/v1/proposals/{id}/decision"), OWNER, Some(json!({ "decision": "approve", "rationale": "Reviewed", "digest": proposed["digest"] }))).await, StatusCode::OK, "A approves")?;
    status(w.rb("GET", &format!("/api/v1/apps/{slug}/tables/apps/records"), OWNER, None).await, StatusCode::NOT_FOUND, "B no longer finds the trial")?;
    Ok(())
}

/// A exports an app and imports the bundle, which makes a proposal. B reads that proposal.
async fn bundle_import(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let bundle = status(w.wa("POST", &format!("/api/v1/apps/{slug}/bundle"), OWNER, Some(json!({}))).await, StatusCode::OK, "A exports the app")?;
    let made = status(w.wa("POST", &format!("/api/v1/workspaces/{WORKSPACE}/bundle-import"), OWNER, Some(json!({ "bundle": bundle }))).await, StatusCode::CREATED, "A imports the bundle")?;
    let id = made["proposal_id"].as_str().ok_or("a proposal id")?;
    let seen = status(w.rb("GET", &format!("/api/v1/proposals/{id}"), OWNER, None).await, StatusCode::OK, "B reads the proposal")?;
    ensure(seen["app_slug"] == made["app_slug"] && seen["status"] == "Pending", format!("B shows the pending proposal A made: {seen}"))
}

/// A makes a trial. B opens it again, which starts its days over.
async fn trial_reopen(w: World) -> Outcome {
    let (id, _) = pending_change(&w).await?;
    let trial = status(w.wa("POST", &format!("/api/v1/proposals/{id}/trial"), OWNER, None).await, StatusCode::CREATED, "A makes the trial")?;
    let trial_id = trial["trial_id"].as_str().ok_or("a trial id")?.to_string();
    let again = status(w.rb("POST", &format!("/api/v1/me/trials/{trial_id}/open"), OWNER, None).await, StatusCode::OK, "B opens A's trial again")?;
    ensure(again["trial_id"] == trial_id.as_str(), format!("B names the trial: {again}"))
}

// --- records -------------------------------------------------------------------------------------------------------------

async fn record_create(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let id = make_record(&w, &slug, "Ada").await?;
    let seen = read_record(&w, &slug, &id).await?.ok_or("B does not find the record A made")?;
    ensure(has(&seen, "Ada"), format!("B shows the record: {seen}"))
}

async fn record_patch(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let id = make_record(&w, &slug, "Before").await?;
    status(w.wa("PATCH", &format!("/api/v1/apps/{slug}/records/{id}"), OWNER, Some(json!({ "data": { "name": "After" }, "version": 1 }))).await, StatusCode::OK, "patch")?;
    let seen = read_record(&w, &slug, &id).await?.ok_or("B does not find the record")?;
    ensure(has(&seen, "After") && !has(&seen, "Before"), format!("B shows the change: {seen}"))
}

async fn record_delete(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let id = make_record(&w, &slug, "Gone").await?;
    ensure(read_record(&w, &slug, &id).await?.is_some(), "B finds the record before it is deleted")?;
    let (s, body) = w.wa("DELETE", &format!("/api/v1/apps/{slug}/records/{id}"), OWNER, None).await;
    ensure(s.is_success(), format!("delete: {s} {body}"))?;
    ensure(read_record(&w, &slug, &id).await?.is_none(), "B no longer finds the record")
}

async fn table_record_create(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let made = status(w.wa("POST", &format!("/api/v1/apps/{slug}/tables/apps/records"), OWNER, Some(json!({ "data": { "name": "Tabled" } }))).await, StatusCode::CREATED, "create")?;
    let id = made["id"].as_str().ok_or("an id")?.to_string();
    let seen = read_record(&w, &slug, &id).await?.ok_or("B does not find the record")?;
    ensure(has(&seen, "Tabled"), format!("B shows the record: {seen}"))
}

async fn table_record_patch(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let id = make_record(&w, &slug, "Before").await?;
    status(w.wa("PATCH", &format!("/api/v1/apps/{slug}/tables/apps/records/{id}"), OWNER, Some(json!({ "data": { "name": "Via table" }, "version": 1 }))).await, StatusCode::OK, "patch")?;
    let seen = read_record(&w, &slug, &id).await?.ok_or("B does not find the record")?;
    ensure(has(&seen, "Via table"), format!("B shows the change: {seen}"))
}

async fn table_record_delete(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let id = make_record(&w, &slug, "Gone").await?;
    let (s, body) = w.wa("DELETE", &format!("/api/v1/apps/{slug}/tables/apps/records/{id}"), OWNER, None).await;
    ensure(s.is_success(), format!("delete: {s} {body}"))?;
    ensure(read_record(&w, &slug, &id).await?.is_none(), "B no longer finds the record")
}

async fn record_restore_version(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let id = make_record(&w, &slug, "First").await?;
    status(w.wa("PATCH", &format!("/api/v1/apps/{slug}/records/{id}"), OWNER, Some(json!({ "data": { "name": "Second" }, "version": 1 }))).await, StatusCode::OK, "patch")?;
    status(
        w.wa("POST", &format!("/api/v1/apps/{slug}/records/{id}/restore"), OWNER, Some(json!({ "to_version": 1, "fields": ["name"], "version": 2 }))).await,
        StatusCode::OK,
        "restore the first version",
    )?;
    let seen = read_record(&w, &slug, &id).await?.ok_or("B does not find the record")?;
    ensure(has(&seen, "First"), format!("B shows the restored value: {seen}"))
}

async fn trash_restore(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let id = make_record(&w, &slug, "Back").await?;
    let (s, body) = w.wa("DELETE", &format!("/api/v1/apps/{slug}/records/{id}"), OWNER, None).await;
    ensure(s.is_success(), format!("delete: {s} {body}"))?;
    ensure(read_record(&w, &slug, &id).await?.is_none(), "B does not find the deleted record")?;
    status(w.wa("POST", &format!("/api/v1/apps/{slug}/trash/{id}/restore"), OWNER, Some(json!({}))).await, StatusCode::OK, "restore from the trash")?;
    ensure(read_record(&w, &slug, &id).await?.is_some(), "B finds the record A restored")
}

async fn history_undo(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let id = make_record(&w, &slug, "Undone").await?;
    let history = status(w.wa("GET", &format!("/api/v1/apps/{slug}/history"), OWNER, None).await, StatusCode::OK, "A reads the history")?;
    let batch = history["entries"][0]["batch_id"].as_str().ok_or_else(|| format!("a batch id in the history: {history}"))?.to_string();
    status(w.wa("POST", &format!("/api/v1/apps/{slug}/history/{batch}/undo"), OWNER, Some(json!({}))).await, StatusCode::OK, "undo the batch")?;
    ensure(read_record(&w, &slug, &id).await?.is_none(), "B no longer finds the record whose creation was undone")
}

// --- comments ------------------------------------------------------------------------------------------------------------

async fn comment_create(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let id = make_record(&w, &slug, "Talked about").await?;
    let note = unique("a note");
    status(w.wa("POST", &format!("/api/v1/apps/{slug}/records/{id}/comments"), OWNER, Some(json!({ "body": note }))).await, StatusCode::CREATED, "comment")?;
    let seen = status(w.rb("GET", &format!("/api/v1/apps/{slug}/records/{id}/comments"), OWNER, None).await, StatusCode::OK, "B lists comments")?;
    ensure(has(&seen, &note), format!("B lists the comment A wrote: {seen}"))
}

async fn comment_edit(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let id = make_record(&w, &slug, "Talked about").await?;
    let made = status(w.wa("POST", &format!("/api/v1/apps/{slug}/records/{id}/comments"), OWNER, Some(json!({ "body": "first" }))).await, StatusCode::CREATED, "comment")?;
    let cid = made["id"].as_str().ok_or("a comment id")?.to_string();
    let edited = unique("edited");
    status(w.wa("PATCH", &format!("/api/v1/apps/{slug}/comments/{cid}"), OWNER, Some(json!({ "body": edited }))).await, StatusCode::OK, "edit")?;
    let seen = status(w.rb("GET", &format!("/api/v1/apps/{slug}/records/{id}/comments"), OWNER, None).await, StatusCode::OK, "B lists comments")?;
    ensure(has(&seen, &edited), format!("B shows the edit: {seen}"))
}

async fn comment_delete(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let id = make_record(&w, &slug, "Talked about").await?;
    let note = unique("doomed");
    let made = status(w.wa("POST", &format!("/api/v1/apps/{slug}/records/{id}/comments"), OWNER, Some(json!({ "body": note }))).await, StatusCode::CREATED, "comment")?;
    let cid = made["id"].as_str().ok_or("a comment id")?.to_string();
    let (s, body) = w.wa("DELETE", &format!("/api/v1/apps/{slug}/comments/{cid}"), OWNER, None).await;
    ensure(s.is_success(), format!("delete: {s} {body}"))?;
    let seen = status(w.rb("GET", &format!("/api/v1/apps/{slug}/records/{id}/comments"), OWNER, None).await, StatusCode::OK, "B lists comments")?;
    ensure(!has(&seen, &note), format!("B no longer lists the deleted comment: {seen}"))
}

// --- views ---------------------------------------------------------------------------------------------------------------

async fn new_view(w: &World, slug: &str, name: &str) -> Result<String, String> {
    let made = status(
        w.wa("POST", &format!("/api/v1/apps/{slug}/views"), OWNER, Some(json!({ "name": name, "kind": "grid", "ownership": "collaborative", "table_id": "apps", "config": {} }))).await,
        StatusCode::CREATED,
        "create a view",
    )?;
    Ok(made["id"].as_str().ok_or("a view id")?.to_string())
}

async fn view_create(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let name = unique("p6view");
    new_view(&w, &slug, &name).await?;
    let seen = status(w.rb("GET", &format!("/api/v1/apps/{slug}/views"), OWNER, None).await, StatusCode::OK, "B lists views")?;
    ensure(has(&seen, &name), format!("B lists the view A made: {seen}"))
}

async fn view_update(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let id = new_view(&w, &slug, &unique("p6view")).await?;
    let renamed = unique("renamed-view");
    status(w.wa("PUT", &format!("/api/v1/apps/{slug}/views/{id}"), OWNER, Some(json!({ "name": renamed, "config": {}, "version": 1 }))).await, StatusCode::OK, "rename")?;
    let seen = status(w.rb("GET", &format!("/api/v1/apps/{slug}/views"), OWNER, None).await, StatusCode::OK, "B lists views")?;
    ensure(has(&seen, &renamed), format!("B shows the new name: {seen}"))
}

async fn view_delete(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let name = unique("doomed-view");
    let id = new_view(&w, &slug, &name).await?;
    let (s, body) = w.wa("DELETE", &format!("/api/v1/apps/{slug}/views/{id}"), OWNER, None).await;
    ensure(s.is_success(), format!("delete: {s} {body}"))?;
    let seen = status(w.rb("GET", &format!("/api/v1/apps/{slug}/views"), OWNER, None).await, StatusCode::OK, "B lists views")?;
    ensure(!has(&seen, &name), format!("B no longer lists the view: {seen}"))
}

async fn view_duplicate(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let name = unique("p6orig");
    let id = new_view(&w, &slug, &name).await?;
    status(w.wa("POST", &format!("/api/v1/apps/{slug}/views/{id}/duplicate"), OWNER, None).await, StatusCode::CREATED, "duplicate")?;
    let seen = status(w.rb("GET", &format!("/api/v1/apps/{slug}/views"), OWNER, None).await, StatusCode::OK, "B lists views")?;
    let copies = seen.to_string().matches(&name).count();
    ensure(copies >= 2, format!("B lists the view and its copy ({copies} mention the name): {seen}"))
}

// --- processes, view-as ----------------------------------------------------------------------------------------------------

/// An app with a rule that waits on the workspace owner, and a record the editor submitted. Returns the slug and the instance id.
pub(super) async fn waiting_instance(w: &World) -> Result<(String, String), String> {
    let slug = make_app(w).await?;
    let rule_id = unique("p6wait");
    let rule = json!({
        "id": rule_id, "name": "Review", "enabled": true, "trigger": "RecordCreated",
        "steps": [{ "id": "decide", "when": [], "kind": { "UserTask": {
            "role": "owner", "prompt": "Decide",
            "approve": [{ "UpdateRecordStatus": { "new_status": "Admitted" } }],
            "reject": [{ "UpdateRecordStatus": { "new_status": "Denied" } }],
            "approver": { "Role": { "name": "owner" } }
        }}}]
    });
    status(w.wa("POST", &format!("/api/v1/apps/{slug}/automations"), OWNER, Some(rule)).await, StatusCode::CREATED, "create the rule")?;
    let made = status(w.wa("POST", &format!("/api/v1/apps/{slug}/records"), EDITOR, Some(json!({ "data": { "_table_id": "apps", "name": "Waiting" } }))).await, StatusCode::CREATED, "the editor submits")?;
    Ok((slug, format!("{rule_id}:{}", made["id"].as_str().ok_or("a record id")?)))
}

async fn process_decide(w: World) -> Outcome {
    let (slug, instance) = waiting_instance(&w).await?;
    let before = status(w.rb("GET", &format!("/api/v1/apps/{slug}/processes"), OWNER, None).await, StatusCode::OK, "B lists instances")?;
    ensure(has(&before, "Waiting"), format!("B sees the instance waiting: {before}"))?;
    status(w.wa("POST", &format!("/api/v1/apps/{slug}/processes/{}/decide", instance.replace(':', "%3A")), OWNER, Some(json!({ "decision": "approve" }))).await, StatusCode::OK, "decide")?;
    let after = status(w.rb("GET", &format!("/api/v1/apps/{slug}/processes"), OWNER, None).await, StatusCode::OK, "B lists instances again")?;
    ensure(has(&after, "Completed") && !has(&after, "Waiting"), format!("B sees the instance decided: {after}"))
}

async fn view_as_start(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let made = status(w.wa("POST", &format!("/api/v1/apps/{slug}/view-as"), OWNER, Some(json!({ "eppn": EDITOR }))).await, StatusCode::OK, "start view-as")?;
    let token = made["token"].as_str().ok_or_else(|| format!("a view-as token: {made}"))?.to_string();
    let (s, body) = World::call_with(&w.b, "GET", &format!("/api/v1/apps/{slug}/records"), &token, None).await;
    ensure(s == StatusCode::OK, format!("B accepts the view-as token A issued: {s} {body}"))
}

async fn view_as_stop(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let made = status(w.wa("POST", &format!("/api/v1/apps/{slug}/view-as"), OWNER, Some(json!({ "eppn": EDITOR }))).await, StatusCode::OK, "start view-as")?;
    let token = made["token"].as_str().ok_or_else(|| format!("a view-as token: {made}"))?.to_string();
    let (s, body) = World::call_with(&w.a, "POST", "/api/v1/auth/stop-view-as", &token, None).await;
    ensure(s.is_success(), format!("stop: {s} {body}"))?;
    let (s, body) = World::call_with(&w.b, "GET", &format!("/api/v1/apps/{slug}/records"), &token, None).await;
    ensure(s == StatusCode::UNAUTHORIZED, format!("B refuses the token after view-as stopped: {s} {body}"))
}

// --- jobs ----------------------------------------------------------------------------------------------------------------

pub(super) async fn job_state(w: &World, id: &str) -> Result<String, String> {
    let seen = status(w.rb("GET", &format!("/api/v1/jobs/{id}"), ADMIN, None).await, StatusCode::OK, "B reads the job")?;
    Ok(seen["state"].as_str().unwrap_or_default().to_string())
}

async fn admin_job_cancel(w: World, root: &'static str) -> Outcome {
    let id = enqueue_job(&w, ADMIN)?;
    ensure(job_state(&w, &id).await? == "queued", "B sees the job queued")?;
    status(w.wa("POST", &format!("{root}/jobs/{id}/cancel"), ADMIN, Some(json!({ "reason": "Not needed" }))).await, StatusCode::OK, "cancel")?;
    let state = job_state(&w, &id).await?;
    ensure(state == "cancelled", format!("B sees the job cancelled, not {state}"))
}

async fn admin_job_retry(w: World, root: &'static str) -> Outcome {
    let id = enqueue_job(&w, ADMIN)?;
    let job = id.clone();
    w.db.run(move |c| c.execute("UPDATE jobs SET state = 'failed', attempts = 3, error = 'broke', finished_at = NOW() WHERE id = $1::text::uuid", &[&job]).unwrap());
    ensure(job_state(&w, &id).await? == "failed", "B sees the job failed")?;
    status(w.wa("POST", &format!("{root}/jobs/{id}/retry"), ADMIN, Some(json!({ "reason": "Try again" }))).await, StatusCode::OK, "retry")?;
    let state = job_state(&w, &id).await?;
    ensure(state == "queued", format!("B sees the job queued again, not {state}"))
}

async fn job_cancel(w: World) -> Outcome {
    let id = enqueue_job(&w, ADMIN)?;
    status(w.wa("POST", &format!("/api/v1/jobs/{id}/cancel"), ADMIN, None).await, StatusCode::OK, "cancel")?;
    let state = job_state(&w, &id).await?;
    ensure(state == "cancelled", format!("B sees the job cancelled, not {state}"))
}

async fn page_call_route(w: World) -> Outcome {
    let slug = super::tools::make_page_app(&w).await?;
    let id = make_record(&w, &slug, "Before").await?;
    status(
        w.wa("POST", &format!("/api/v1/apps/{slug}/pages/board/call"), OWNER, Some(json!({ "op": "update", "table_id": "apps", "record_id": id, "version": 1, "data": { "name": "By route" } }))).await,
        StatusCode::OK,
        "update through a page",
    )?;
    status(w.wa("POST", &format!("/api/v1/apps/{slug}/pages/board/call"), OWNER, Some(json!({ "op": "settings_set", "key": "k", "value": "route-kept" }))).await, StatusCode::OK, "set")?;
    let seen = status(w.rb("POST", &format!("/api/v1/apps/{slug}/pages/board/call"), OWNER, Some(json!({ "op": "get", "table_id": "apps", "record_id": id }))).await, StatusCode::OK, "B reads through the page")?;
    ensure(has(&seen, "By route"), format!("B shows the change: {seen}"))?;
    let setting = status(w.rb("POST", &format!("/api/v1/apps/{slug}/pages/board/call"), OWNER, Some(json!({ "op": "settings_get", "key": "k" }))).await, StatusCode::OK, "get")?;
    ensure(has(&setting, "route-kept"), format!("B reads the setting: {setting}"))
}
