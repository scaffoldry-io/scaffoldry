//! Separation of duties (access model phase 10). The setting `access.conflicts` lists pairs of holders that one person may
//! not hold at once. A holder is a position, or a role on a domain. The shape is `governance/schema/access-conflicts.schema.json`.

use serde_json::Value;

/// One thing a person can hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Holder {
    Position(String),
    Role { role: String, domain: Option<String> },
}

impl Holder {
    /// The words the refusal uses.
    pub fn words(&self) -> String {
        match self {
            Holder::Position(k) => format!("the position '{k}'"),
            Holder::Role { role, domain: Some(d) } => format!("the role '{role}' on '{d}'"),
            Holder::Role { role, domain: None } => format!("the role '{role}'"),
        }
    }

    fn parse(v: &Value) -> Result<Holder, String> {
        let obj = v.as_object().ok_or("a holder is an object")?;
        let text = |k: &str| obj.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
        if obj.keys().any(|k| !["position", "role", "domain"].contains(&k.as_str())) {
            return Err("a holder has only position, or role and domain".to_string());
        }
        match (text("position"), text("role"), text("domain")) {
            (Some(p), None, None) => Ok(Holder::Position(p)),
            (None, Some(role), domain) => Ok(Holder::Role { role, domain }),
            _ => Err("a holder is a position, or a role with an optional domain".to_string()),
        }
    }
}

/// Two holders one person may not hold together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    pub a: Holder,
    pub b: Holder,
}

impl Pair {
    pub fn words(&self) -> String {
        format!("{} and {}", self.a.words(), self.b.words())
    }
}

/// The pairs in the setting. An error names the first bad entry.
pub fn parse(setting: &Value) -> Result<Vec<Pair>, String> {
    let list = setting.as_array().ok_or("access.conflicts is a list of pairs")?;
    list.iter()
        .enumerate()
        .map(|(i, p)| {
            let obj = p.as_object().filter(|o| o.len() == 2).ok_or(format!("[{i}] needs exactly a and b"))?;
            let side = |k: &str| Holder::parse(obj.get(k).ok_or(format!("[{i}] needs {k}"))?).map_err(|e| format!("[{i}].{k}: {e}"));
            Ok(Pair { a: side("a")?, b: side("b")? })
        })
        .collect()
}

/// The first pair that `new` would complete, given what the person holds now.
pub fn completes<'a>(pairs: &'a [Pair], held: &[Holder], new: &Holder) -> Option<&'a Pair> {
    pairs.iter().find(|p| (&p.a == new && held.contains(&p.b)) || (&p.b == new && held.contains(&p.a)))
}

/// Every pair a person holds both sides of.
pub fn existing<'a>(pairs: &'a [Pair], held: &[Holder]) -> Vec<&'a Pair> {
    pairs.iter().filter(|p| held.contains(&p.a) && held.contains(&p.b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pos(k: &str) -> Holder {
        Holder::Position(k.to_string())
    }

    #[test]
    fn parses_both_kinds_and_refuses_the_rest() {
        let ok = parse(&json!([{ "a": { "position": "dean" }, "b": { "role": "admin", "domain": "research" } }])).unwrap();
        assert_eq!(ok[0].a, pos("dean"));
        assert_eq!(ok[0].b, Holder::Role { role: "admin".into(), domain: Some("research".into()) });
        for bad in [json!({}), json!([{ "a": { "position": "x" } }]), json!([{ "a": {}, "b": { "position": "y" } }]), json!([{ "a": { "position": "x", "role": "admin" }, "b": { "position": "y" } }]), json!([{ "a": { "position": "x" }, "b": { "position": "y" }, "c": 1 }])] {
            assert!(parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_new_holder_completes_a_pair_from_either_side() {
        let pairs = parse(&json!([{ "a": { "position": "dean" }, "b": { "position": "auditor" } }])).unwrap();
        assert!(completes(&pairs, &[pos("dean")], &pos("auditor")).is_some());
        assert!(completes(&pairs, &[pos("auditor")], &pos("dean")).is_some());
        assert!(completes(&pairs, &[pos("dean")], &pos("chair")).is_none());
        assert!(completes(&pairs, &[], &pos("auditor")).is_none());
    }

    #[test]
    fn a_role_matches_only_its_own_domain() {
        let pairs = parse(&json!([{ "a": { "position": "dean" }, "b": { "role": "admin", "domain": "research" } }])).unwrap();
        let on = |d: &str| Holder::Role { role: "admin".into(), domain: Some(d.into()) };
        assert!(completes(&pairs, &[pos("dean")], &on("research")).is_some());
        assert!(completes(&pairs, &[pos("dean")], &on("engagement")).is_none());
        assert_eq!(existing(&pairs, &[pos("dean"), on("research")]).len(), 1);
        assert!(existing(&pairs, &[pos("dean")]).is_empty());
    }
}
