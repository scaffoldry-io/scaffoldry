//! Database Schema Migration & Persistence Integration Test
//! Tests PostgreSQL 17 relational schema, CEDS mappings, and custom domain DNS aliasing.

use postgres::{Client, NoTls};
use serde_json::json;

const SCHEMA_SQL: &str = include_str!("../migrations/0001_initial_schema.sql");

fn connect_db() -> Result<Client, postgres::Error> {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string()
    });
    Client::connect(&url, NoTls)
}

#[test]
fn test_database_schema_migration_and_domain_records() {
    let mut client = match connect_db() {
        Ok(c) => c,
        Err(e) => {
            if std::env::var("CI").is_ok() {
                panic!("CI requires PostgreSQL on 127.0.0.1:5433 but connection failed: {}", e);
            } else {
                eprintln!("Skipping database integration test (PostgreSQL not reachable: {})", e);
                return;
            }
        }
    };

    // 1. Apply Schema Migration
    client
        .batch_execute(SCHEMA_SQL)
        .expect("Failed to execute 0001_initial_schema.sql");

    // 2. Verify all core tables exist in information_schema
    let expected_tables = vec![
        "organizations",
        "persons",
        "roles",
        "academic_plans",
        "facilities",
        "apps",
        "records",
    ];

    for table in expected_tables {
        let row = client
            .query_one(
                "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = 'public' AND table_name = $1",
                &[&table],
            )
            .unwrap_or_else(|e| panic!("Error checking existence of table {}: {}", table, e));
        let count: i64 = row.get(0);
        assert_eq!(count, 1, "Table {} must exist after migration", table);
    }

    // 3. Clean up any previous test fixture with our unique test code
    let test_code = "TST-ORG-4421";
    let test_domain = "bio-inventory.science.state.edu";
    client.execute("DELETE FROM organizations WHERE code = $1", &[&test_code]).ok();

    // 4. Insert CEDS Organization (Higher Education / Academic Subdivision)
    let org_row = client.query_one(
        "INSERT INTO organizations (name, code, org_type, ipeds_unit_id, herm_domain)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id",
        &[
            &"Department of Biology",
            &test_code,
            &"AcademicSubdivision",
            &"234076",
            &"LearningAndTeaching",
        ],
    ).expect("Failed to insert test organization");
    let org_id: uuid::Uuid = org_row.get(0);

    // 5. Insert Application with DNS Aliasing support
    let app_row = client.query_one(
        "INSERT INTO apps (organization_id, slug, title, herm_capability_id, custom_domain, custom_domain_verified, is_published)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id",
        &[
            &org_id,
            &"bio-lab-inventory",
            &"Biology Lab Equipment Inventory",
            &"2.2.3",
            &test_domain,
            &true,
            &true,
        ],
    ).expect("Failed to insert test app with custom domain");
    let app_id: uuid::Uuid = app_row.get(0);

    // 6. Query by Custom Domain DNS Alias
    let domain_query_row = client.query_one(
        "SELECT id, slug, title, custom_domain FROM apps WHERE custom_domain = $1",
        &[&test_domain],
    ).expect("Should find app by its DNS custom domain alias");
    let matched_id: uuid::Uuid = domain_query_row.get(0);
    assert_eq!(matched_id, app_id);

    // 7. Insert Record with dynamic JSONB data and CEDS element mappings
    let record_data = json!({
        "item_name": "Spectrophotometer Bio-2000",
        "serial_number": "SN-998234-X",
        "room": "Gilmer 204"
    });
    let ceds_mapping = json!({
        "facility_element": "000185",
        "equipment_category": "ResearchLab"
    });

    let rec_row = client.query_one(
        "INSERT INTO records (app_id, data, ceds_mapping, is_ferpa_sensitive)
         VALUES ($1, $2, $3, $4)
         RETURNING id, data, ceds_mapping",
        &[&app_id, &record_data, &ceds_mapping, &false],
    ).expect("Failed to insert record with CEDS metadata");
    let rec_id: uuid::Uuid = rec_row.get(0);
    let returned_data: serde_json::Value = rec_row.get(1);
    let returned_ceds: serde_json::Value = rec_row.get(2);

    assert_eq!(returned_data["item_name"], "Spectrophotometer Bio-2000");
    assert_eq!(returned_ceds["facility_element"], "000185");

    // 8. Clean up test record and cascade delete
    client.execute("DELETE FROM organizations WHERE id = $1", &[&org_id]).expect("Cascade delete should succeed");

    // Verify app and record were cascade deleted
    let remaining_apps: i64 = client.query_one("SELECT COUNT(*) FROM apps WHERE id = $1", &[&app_id]).unwrap().get(0);
    assert_eq!(remaining_apps, 0, "App should be cascaded when organization is deleted");

    let remaining_records: i64 = client.query_one("SELECT COUNT(*) FROM records WHERE id = $1", &[&rec_id]).unwrap().get(0);
    assert_eq!(remaining_records, 0, "Record should be cascaded when app is deleted");
}
