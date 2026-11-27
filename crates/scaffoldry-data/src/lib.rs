//! The data layer: the one place that issues SQL against the record and label tables.
//!
//! It owns `dataset_records` (with its `unit_id` and `tags` columns; `catalog` reads only counts and a date from it), `record_index`, `record_keys`, `data_labels` and `label_changes` (the proposals of data layer phase 6, which name rows), and `row_shares` with `row_share_rows` (data layer phase 10: a share names rows and a person), and `record_audience` (data layer phase 3b: the names in a row's audience fields). Every function takes a `&mut` transaction or client and
//! plain types, and returns plain types. It decides nothing about who may see a row. Access checks, history, links and
//! jobs stay in the server, which calls these functions. `governance/scripts/audit-architecture.py` fails any other
//! `.rs` file that names these tables, except the explicit allow-list kept there.

pub mod audience;
pub mod catalog;
pub mod changes;
pub mod gate;
pub mod index;
pub mod keys;
pub mod labels;
pub mod records;
pub mod shares;
pub mod specs;
pub mod units;
