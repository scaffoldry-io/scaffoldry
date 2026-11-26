//! The data layer: the one place that issues SQL against the record and label tables.
//!
//! It owns `dataset_records` (with its `unit_id` column; `catalog` reads only counts and a date from it), `record_index`, `record_keys` and `data_labels`. Every function takes a `&mut` transaction or client and
//! plain types, and returns plain types. It decides nothing about who may see a row. Access checks, history, links and
//! jobs stay in the server, which calls these functions. `governance/scripts/audit-architecture.py` fails any other
//! `.rs` file that names these tables, except the explicit allow-list kept there.

pub mod catalog;
pub mod index;
pub mod keys;
pub mod labels;
pub mod records;
pub mod specs;
pub mod units;
