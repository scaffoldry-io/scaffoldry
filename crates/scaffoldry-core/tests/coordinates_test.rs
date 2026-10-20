//! Access model phase 3: every object has coordinates. `effective_coordinates` is a pure function. Each level takes its
//! container's value unless it narrows it, and a level that would widen it is refused.

use scaffoldry_core::coordinates::{effective_coordinates, sensitivity_rank, Coordinates, Places, ROOT_DOMAIN};
use std::collections::HashMap;

/// A small world. Units: `acme` is the root of `academic`; `science` and `arts` are under it; `physics` is under
/// `science`. Domains: `institution` is the root; `research` is under it; `grants` is under `research`; `teaching` is
/// under `institution`. A second hierarchy, `sites`, has one root `campus` and a unit `lab` under it.
struct World {
    unit_parent: HashMap<(&'static str, &'static str), Option<&'static str>>,
    domain_parent: HashMap<&'static str, Option<&'static str>>,
}

impl World {
    fn new() -> World {
        let mut unit_parent = HashMap::new();
        for (h, u, p) in [
            ("academic", "acme", None),
            ("academic", "science", Some("acme")),
            ("academic", "arts", Some("acme")),
            ("academic", "physics", Some("science")),
            ("sites", "campus", None),
            ("sites", "lab", Some("campus")),
        ] {
            unit_parent.insert((h, u), p);
        }
        let mut domain_parent = HashMap::new();
        for (d, p) in [("institution", None), ("research", Some("institution")), ("grants", Some("research")), ("teaching", Some("institution"))] {
            domain_parent.insert(d, p);
        }
        World { unit_parent, domain_parent }
    }
}

impl Places for World {
    fn unit_within(&self, hierarchy: &str, unit: &str, container: &str) -> bool {
        let mut current = self.unit_parent.get_key_value(&(hierarchy, unit)).map(|(k, _)| k.1);
        while let Some(u) = current {
            if u == container {
                return true;
            }
            current = self.unit_parent.get(&(hierarchy, u)).copied().flatten();
        }
        false
    }

    fn domain_within(&self, domain: &str, container: &str) -> bool {
        let mut current = self.domain_parent.get_key_value(domain).map(|(k, _)| *k);
        while let Some(d) = current {
            if d == container {
                return true;
            }
            current = self.domain_parent.get(d).copied().flatten();
        }
        false
    }

    fn hierarchies(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.unit_parent.keys().map(|(h, _)| h.to_string()).collect();
        keys.sort();
        keys.dedup();
        keys
    }

    fn root_unit(&self, hierarchy: &str) -> Option<String> {
        self.unit_parent.iter().find(|((h, _), p)| *h == hierarchy && p.is_none()).map(|((_, u), _)| u.to_string())
    }
}

fn units(pairs: &[(&str, &str)]) -> Coordinates {
    Coordinates { units: pairs.iter().map(|(h, u)| (h.to_string(), u.to_string())).collect(), ..Coordinates::default() }
}

fn domain(d: &str) -> Coordinates {
    Coordinates { domain: Some(d.to_string()), ..Coordinates::default() }
}

fn sensitivity(s: &str) -> Coordinates {
    Coordinates { sensitivity: Some(s.to_string()), ..Coordinates::default() }
}

#[test]
fn the_ladder_is_public_directory_internal_restricted() {
    let ranks: Vec<_> = ["Public", "Directory", "Internal", "Restricted"].iter().map(|s| sensitivity_rank(s)).collect();
    assert_eq!(ranks, [Some(0), Some(1), Some(2), Some(3)]);
    assert_eq!(sensitivity_rank("Secret"), None);
}

// 1. Inherit, narrow, and refuse to widen, on each axis.
#[test]
fn a_level_inherits_narrows_or_is_refused_on_every_axis() {
    let w = World::new();

    // Nothing set anywhere: the workspace's missing values are each axis's root and Internal.
    let none = effective_coordinates(None, None, None, None, &w).expect("nothing set");
    assert_eq!(none.domain, ROOT_DOMAIN);
    assert_eq!(none.sensitivity, "Internal");
    assert_eq!(none.units.get("academic").map(String::as_str), Some("acme"), "the root of each hierarchy that has one");

    // Units.
    let ws = units(&[("academic", "science")]);
    let inherited = effective_coordinates(Some(&ws), None, None, None, &w).unwrap();
    assert_eq!(inherited.units["academic"], "science", "an app with no unit takes the workspace's");
    let narrowed = effective_coordinates(Some(&ws), Some(&units(&[("academic", "physics")])), None, None, &w).unwrap();
    assert_eq!(narrowed.units["academic"], "physics", "a unit below the container's narrows it");
    let same = effective_coordinates(Some(&ws), Some(&units(&[("academic", "science")])), None, None, &w).unwrap();
    assert_eq!(same.units["academic"], "science", "the container's own unit is allowed");
    let widened = effective_coordinates(Some(&ws), Some(&units(&[("academic", "acme")])), None, None, &w).unwrap_err();
    assert_eq!((widened[0].level, widened[0].axis.as_str(), widened[0].value.as_str(), widened[0].container.as_str()), ("app", "unit:academic", "acme", "science"));
    let sideways = effective_coordinates(Some(&ws), Some(&units(&[("academic", "arts")])), None, None, &w).unwrap_err();
    assert_eq!(sideways[0].value, "arts", "a sibling is not below the container's unit");
    // A table below an app that is already narrowed.
    let table_up = effective_coordinates(Some(&ws), Some(&units(&[("academic", "physics")])), Some(&units(&[("academic", "science")])), None, &w).unwrap_err();
    assert_eq!((table_up[0].level, table_up[0].value.as_str(), table_up[0].container.as_str()), ("table", "science", "physics"));

    // A second hierarchy is its own axis.
    let two = effective_coordinates(Some(&units(&[("academic", "science"), ("sites", "campus")])), Some(&units(&[("sites", "lab")])), None, None, &w).unwrap();
    assert_eq!((two.units["academic"].as_str(), two.units["sites"].as_str()), ("science", "lab"));

    // Domains.
    let wd = domain("research");
    assert_eq!(effective_coordinates(Some(&wd), None, None, None, &w).unwrap().domain, "research");
    assert_eq!(effective_coordinates(Some(&wd), Some(&domain("grants")), None, None, &w).unwrap().domain, "grants");
    let up = effective_coordinates(Some(&wd), Some(&domain("institution")), None, None, &w).unwrap_err();
    assert_eq!((up[0].axis.as_str(), up[0].value.as_str(), up[0].container.as_str()), ("domain", "institution", "research"));
    let across = effective_coordinates(Some(&wd), Some(&domain("teaching")), None, None, &w).unwrap_err();
    assert_eq!(across[0].value, "teaching");

    // Sensitivity: the same or higher.
    let ws_s = sensitivity("Directory");
    assert_eq!(effective_coordinates(Some(&ws_s), None, None, None, &w).unwrap().sensitivity, "Directory");
    assert_eq!(effective_coordinates(Some(&ws_s), Some(&sensitivity("Restricted")), None, None, &w).unwrap().sensitivity, "Restricted");
    assert_eq!(effective_coordinates(Some(&ws_s), Some(&sensitivity("Directory")), None, None, &w).unwrap().sensitivity, "Directory");
    let lower = effective_coordinates(Some(&ws_s), Some(&sensitivity("Public")), None, None, &w).unwrap_err();
    assert_eq!((lower[0].axis.as_str(), lower[0].value.as_str(), lower[0].container.as_str()), ("sensitivity", "Public", "Directory"));
    // The default Internal is a container's value too: an app cannot go below it unless the workspace says so.
    let below_default = effective_coordinates(None, Some(&sensitivity("Public")), None, None, &w).unwrap_err();
    assert_eq!(below_default[0].container, "Internal");

    // A field has only a sensitivity, taken against its table's.
    let table = sensitivity("Restricted");
    assert_eq!(effective_coordinates(None, None, Some(&table), Some("Restricted"), &w).unwrap().sensitivity, "Restricted");
    let field_lower = effective_coordinates(None, None, Some(&table), Some("Internal"), &w).unwrap_err();
    assert_eq!((field_lower[0].level, field_lower[0].value.as_str(), field_lower[0].container.as_str()), ("field", "Internal", "Restricted"));
    assert_eq!(effective_coordinates(None, None, None, Some("Restricted"), &w).unwrap().sensitivity, "Restricted", "a field may be higher than its table");

    // Every widening is reported, not only the first.
    let both = effective_coordinates(Some(&ws), Some(&Coordinates { units: [("academic".to_string(), "acme".to_string())].into(), sensitivity: Some("Public".to_string()), ..Coordinates::default() }), None, None, &w).unwrap_err();
    assert_eq!(both.len(), 2);
    // A word that is not on the ladder is refused by name.
    assert!(effective_coordinates(None, Some(&sensitivity("Secret")), None, None, &w).is_err());
}
