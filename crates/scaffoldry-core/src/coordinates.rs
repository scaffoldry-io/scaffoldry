//! Coordinates: where an object sits (access model phase 3).
//!
//! A workspace, an app, a table and a field each have a place on three axes: a unit in each hierarchy, a domain, and a
//! sensitivity. A child takes its container's value unless it narrows it. It may name a unit that is its container's or
//! below it, a domain that is its container's or below it, and a sensitivity that is the same or higher. A child that
//! would widen is refused, and the refusal names the level, the axis, the value, and the container's value.
//!
//! Everything here is pure. The ancestry of units and domains comes in through `Places`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// The sensitivity ladder, lowest first.
pub const SENSITIVITY_LEVELS: [&str; 4] = ["Public", "Directory", "Internal", "Restricted"];

/// What a workspace has when it names no sensitivity.
pub const DEFAULT_SENSITIVITY: &str = "Internal";

/// The root of the domain tree. It is what a workspace has when it names no domain.
pub const ROOT_DOMAIN: &str = "institution";

/// The place of a level in the ladder. `None` for a word that is not on it.
pub fn sensitivity_rank(level: &str) -> Option<u8> {
    SENSITIVITY_LEVELS.iter().position(|l| *l == level).map(|i| i as u8)
}

/// The coordinates one level names for itself. A value left out is the container's.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coordinates {
    /// A unit id for each hierarchy key.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub units: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensitivity: Option<String>,
    /// A sealed workspace stops every grant anchored above it (access model phase 7). It is not inherited: an app inside
    /// a sealed workspace is not itself sealed, and says so in its own manifest when it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sealed: Option<bool>,
}

impl Coordinates {
    pub fn is_empty(&self) -> bool {
        self.units.is_empty() && self.domain.is_none() && self.sensitivity.is_none() && self.sealed.is_none()
    }

    /// The stored form: one `(axis, value)` row for each coordinate. An axis is `unit:<hierarchy key>`, `domain`, or
    /// `sensitivity`.
    pub fn to_axes(&self) -> Vec<(String, String)> {
        let mut rows: Vec<(String, String)> = self.units.iter().map(|(h, u)| (format!("unit:{h}"), u.clone())).collect();
        if let Some(d) = &self.domain {
            rows.push(("domain".to_string(), d.clone()));
        }
        if let Some(s) = &self.sensitivity {
            rows.push(("sensitivity".to_string(), s.clone()));
        }
        if let Some(sealed) = self.sealed {
            rows.push(("sealed".to_string(), sealed.to_string()));
        }
        rows
    }

    /// The same from stored rows. A row with an axis nobody knows is ignored.
    pub fn from_axes(rows: impl IntoIterator<Item = (String, String)>) -> Coordinates {
        let mut out = Coordinates::default();
        for (axis, value) in rows {
            if let Some(h) = axis.strip_prefix("unit:") {
                out.units.insert(h.to_string(), value);
            } else if axis == "domain" {
                out.domain = Some(value);
            } else if axis == "sensitivity" {
                out.sensitivity = Some(value);
            } else if axis == "sealed" {
                out.sealed = Some(value == "true");
            }
        }
        out
    }
}

/// The ancestry the rules need. A unit or a domain is within itself.
pub trait Places {
    /// Whether `unit` is `container` or below it in the hierarchy. False for a unit the hierarchy does not hold.
    fn unit_within(&self, hierarchy: &str, unit: &str, container: &str) -> bool;
    /// Whether `domain` is `container` or below it. False for a domain that does not exist.
    fn domain_within(&self, domain: &str, container: &str) -> bool;
    /// The keys of the hierarchies that exist.
    fn hierarchies(&self) -> Vec<String>;
    /// The root unit of a hierarchy.
    fn root_unit(&self, hierarchy: &str) -> Option<String>;
}

/// The coordinates an object has once its containers are taken into account. Every axis has a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effective {
    pub units: BTreeMap<String, String>,
    pub domain: String,
    pub sensitivity: String,
}

impl Effective {
    /// A workspace that names nothing: the root of each hierarchy, the root domain, and `Internal`.
    pub fn root(places: &dyn Places) -> Effective {
        let units = places.hierarchies().into_iter().filter_map(|h| places.root_unit(&h).map(|u| (h, u))).collect();
        Effective { units, domain: ROOT_DOMAIN.to_string(), sensitivity: DEFAULT_SENSITIVITY.to_string() }
    }
}

/// One coordinate that would widen its container's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Widening {
    /// `workspace`, `app`, `table`, `field`, or a path the caller gives, such as `table 'applicants'`.
    pub level: &'static str,
    /// `unit:<hierarchy key>`, `domain`, or `sensitivity`.
    pub axis: String,
    pub value: String,
    pub container: String,
    /// True when the value is not a level on the ladder at all, so there is no container's value to compare.
    pub unknown: bool,
}

impl fmt::Display for Widening {
    /// The words without the path: the caller puts the path of the object in front, such as `table 'people' field 'gpa'`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.unknown {
            write!(f, "{} '{}' is not one of {}", self.axis, self.value, SENSITIVITY_LEVELS.join(", "))
        } else {
            write!(f, "{} '{}' would widen its container's '{}'", self.axis, self.value, self.container)
        }
    }
}

/// A workspace has no container. What it names it holds, and what it leaves out is the root of each axis and `Internal`.
/// A sensitivity that is not on the ladder is refused.
pub fn workspace_effective(own: Option<&Coordinates>, places: &dyn Places) -> Result<Effective, Vec<Widening>> {
    let mut out = Effective::root(places);
    let Some(own) = own else { return Ok(out) };
    for (hierarchy, unit) in &own.units {
        out.units.insert(hierarchy.clone(), unit.clone());
    }
    if let Some(domain) = &own.domain {
        out.domain = domain.clone();
    }
    if let Some(s) = &own.sensitivity {
        if sensitivity_rank(s).is_none() {
            return Err(vec![Widening { level: "workspace", axis: "sensitivity".to_string(), value: s.clone(), container: String::new(), unknown: true }]);
        }
        out.sensitivity = s.clone();
    }
    Ok(out)
}

/// One level against its container: the level's own coordinates taken over the container's effective ones.
pub fn narrow(container: &Effective, own: Option<&Coordinates>, level: &'static str, places: &dyn Places) -> Result<Effective, Vec<Widening>> {
    let mut out = container.clone();
    let mut refused = Vec::new();
    let Some(own) = own else { return Ok(out) };
    for (hierarchy, unit) in &own.units {
        match container.units.get(hierarchy) {
            Some(above) if !places.unit_within(hierarchy, unit, above) => {
                refused.push(Widening { level, axis: format!("unit:{hierarchy}"), value: unit.clone(), container: above.clone(), unknown: false });
            }
            _ => {
                out.units.insert(hierarchy.clone(), unit.clone());
            }
        }
    }
    if let Some(domain) = &own.domain {
        if places.domain_within(domain, &container.domain) {
            out.domain = domain.clone();
        } else {
            refused.push(Widening { level, axis: "domain".to_string(), value: domain.clone(), container: container.domain.clone(), unknown: false });
        }
    }
    if let Some(level_name) = &own.sensitivity {
        out.sensitivity = narrow_sensitivity(&container.sensitivity, level_name, level, &mut refused, out.sensitivity.clone());
    }
    if refused.is_empty() { Ok(out) } else { Err(refused) }
}

/// A sensitivity against the container's. A word off the ladder, or one below the container's, is refused.
fn narrow_sensitivity(container: &str, own: &str, level: &'static str, refused: &mut Vec<Widening>, keep: String) -> String {
    match (sensitivity_rank(own), sensitivity_rank(container)) {
        (Some(a), Some(b)) if a >= b => own.to_string(),
        (None, _) => {
            refused.push(Widening { level, axis: "sensitivity".to_string(), value: own.to_string(), container: String::new(), unknown: true });
            keep
        }
        _ => {
            refused.push(Widening { level, axis: "sensitivity".to_string(), value: own.to_string(), container: container.to_string(), unknown: false });
            keep
        }
    }
}

/// The coordinates of a field: the workspace's, narrowed by the app's, then the table's, then the field's own
/// sensitivity. Each level that is `None` takes its container's. Every widening is returned, not only the first.
pub fn effective_coordinates(
    workspace: Option<&Coordinates>,
    app: Option<&Coordinates>,
    table: Option<&Coordinates>,
    field_sensitivity: Option<&str>,
    places: &dyn Places,
) -> Result<Effective, Vec<Widening>> {
    let mut refused = Vec::new();
    let mut here = workspace_effective(workspace, places)?;
    for (own, level) in [(app, "app"), (table, "table")] {
        match narrow(&here, own, level, places) {
            Ok(next) => here = next,
            Err(mut r) => refused.append(&mut r),
        }
    }
    if let Some(s) = field_sensitivity {
        let own = Coordinates { sensitivity: Some(s.to_string()), ..Coordinates::default() };
        match narrow(&here, Some(&own), "field", places) {
            Ok(next) => here = next,
            Err(mut r) => refused.append(&mut r),
        }
    }
    if refused.is_empty() { Ok(here) } else { Err(refused) }
}
