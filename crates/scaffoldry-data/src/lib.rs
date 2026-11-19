//! The data layer: the one place that issues SQL against the record and label tables.
//!
//! It owns `dataset_records`, `record_index` and `data_labels`. Every function takes a `&mut` transaction or client and
//! plain types, and returns plain types. It decides nothing about who may see a row. Access checks, history, links and
//! jobs stay in the server, which calls these functions. `governance/scripts/audit-architecture.py` fails any other
//! `.rs` file that names these tables, except the explicit allow-list kept there.

pub mod index;
pub mod labels;
pub mod records;
