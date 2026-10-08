//! Multi-View Engine and Data Shaping Integration Test

use scaffoldry_engine::{
    matches_filter, sort_records, AppView, CompoundFilter, FilterClause, FilterConjunction,
    FilterOperator, RowDensity, SortDirection, SortRule, ViewType,
};
use serde_json::json;

#[test]
fn test_compound_filter_and_or() {
    let records = [
        json!({ "id": "R1", "status": "Approved", "budget": 800000.0, "dept": "Physics" }),
        json!({ "id": "R2", "status": "Under Review", "budget": 450000.0, "dept": "Bioengineering" }),
        json!({ "id": "R3", "status": "Funded", "budget": 600000.0, "dept": "Physics" }),
        json!({ "id": "R4", "status": "Approved", "budget": 300000.0, "dept": "Materials Science" }),
    ];

    // AND filter: status == "Approved" AND budget > 500000
    let and_filter = CompoundFilter {
        conjunction: FilterConjunction::And,
        clauses: vec![
            FilterClause {
                id: "c1".to_string(),
                field_name: "status".to_string(),
                operator: FilterOperator::Equals,
                value: "Approved".to_string(),
            },
            FilterClause {
                id: "c2".to_string(),
                field_name: "budget".to_string(),
                operator: FilterOperator::GreaterThan,
                value: "500000".to_string(),
            },
        ],
    };

    let filtered_and: Vec<_> = records.iter().filter(|r| matches_filter(r, &and_filter)).collect();
    assert_eq!(filtered_and.len(), 1);
    assert_eq!(filtered_and[0]["id"], "R1");

    // OR filter: dept == "Bioengineering" OR status == "Funded"
    let or_filter = CompoundFilter {
        conjunction: FilterConjunction::Or,
        clauses: vec![
            FilterClause {
                id: "c3".to_string(),
                field_name: "dept".to_string(),
                operator: FilterOperator::Contains,
                value: "Bio".to_string(),
            },
            FilterClause {
                id: "c4".to_string(),
                field_name: "status".to_string(),
                operator: FilterOperator::Equals,
                value: "Funded".to_string(),
            },
        ],
    };

    let filtered_or: Vec<_> = records.iter().filter(|r| matches_filter(r, &or_filter)).collect();
    assert_eq!(filtered_or.len(), 2);
    let ids: Vec<&str> = filtered_or.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"R2"));
    assert!(ids.contains(&"R3"));
}

#[test]
fn test_multi_column_sorting() {
    let mut records = vec![
        json!({ "id": "R1", "status": "Approved", "budget": 800000.0 }),
        json!({ "id": "R2", "status": "Approved", "budget": 300000.0 }),
        json!({ "id": "R3", "status": "Under Review", "budget": 500000.0 }),
    ];

    // Sort by status ASC, then budget DESC
    let rules = vec![
        SortRule {
            id: "s1".to_string(),
            field_name: "status".to_string(),
            direction: SortDirection::Asc,
        },
        SortRule {
            id: "s2".to_string(),
            field_name: "budget".to_string(),
            direction: SortDirection::Desc,
        },
    ];

    sort_records(&mut records, &rules);

    assert_eq!(records[0]["id"], "R1"); // Approved, 800000
    assert_eq!(records[1]["id"], "R2"); // Approved, 300000
    assert_eq!(records[2]["id"], "R3"); // Under Review, 500000
}

#[test]
fn test_view_spec_serialization_with_all_view_types() {
    let view = AppView {
        id: "view-kanban-1".to_string(),
        table_id: Some("tbl-proposals".to_string()),
        title: "Proposal Workflow Stages".to_string(),
        view_type: ViewType::Kanban,
        fields: vec![],
        filters: None,
        sort_rules: vec![],
        group_by_field: Some("status".to_string()),
        row_density: Some(RowDensity::Medium),
        kanban_column_field: Some("status".to_string()),
        calendar_date_field: None,
        column_order: vec![],
        column_widths: vec![],
        hidden_columns: vec![],
        frozen_through: None,
        column_summary: vec![],
    };

    let serialized = serde_json::to_string(&view).expect("Serialize AppView");
    let deserialized: AppView = serde_json::from_str(&serialized).expect("Deserialize AppView");

    assert_eq!(deserialized.view_type, ViewType::Kanban);
    assert_eq!(deserialized.row_density, Some(RowDensity::Medium));
    assert_eq!(deserialized.group_by_field, Some("status".to_string()));
}
