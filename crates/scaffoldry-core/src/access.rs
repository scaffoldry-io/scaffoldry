//! Access rules as data: a predicate over a record and a person, and the scope it gives.
//!
//! A rule is a `Pred`. `substitute` replaces the person's values and folds constants, so a rule
//! becomes `AllowAll`, `DenyAll`, or a `Mixed` predicate over record fields only. The engine
//! evaluates a `Mixed` predicate in Rust (`eval`) and compiles it to SQL (`to_sql`). A test requires
//! the two to agree.
//!
//! A `Cmp` names a record field. A field written `person.{attr}` (`person.department`) names the
//! person's own value instead, so a rule can compare two things about the person. After
//! `substitute` no `Person` operand and no `person.` field is left.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The most characters of a text value that are compared. The index stores the same.
pub const MAX_TEXT: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    Eq,
    Ne,
    In,
    IsEmpty,
    IsNotEmpty,
    Gt,
    Lt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonAttr {
    Address,
    Department,
    Affiliation,
    UnitIds,
    PositionKeys,
    GroupIds,
}

impl PersonAttr {
    pub fn parse(name: &str) -> Option<PersonAttr> {
        match name {
            "address" => Some(PersonAttr::Address),
            "department" => Some(PersonAttr::Department),
            "affiliation" => Some(PersonAttr::Affiliation),
            "unit_ids" => Some(PersonAttr::UnitIds),
            "position_keys" => Some(PersonAttr::PositionKeys),
            "group_ids" => Some(PersonAttr::GroupIds),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operand {
    Literal(Value),
    Person(PersonAttr),
}

fn blank_operand() -> Operand {
    Operand::Literal(Value::Null)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pred {
    True,
    Cmp {
        field: String,
        op: Op,
        /// `IsEmpty` and `IsNotEmpty` take no value.
        #[serde(default = "blank_operand")]
        value: Operand,
    },
    And(Vec<Pred>),
    Or(Vec<Pred>),
    Not(Box<Pred>),
}

/// What a rule lets a person see. `Mixed` holds a predicate over record fields only.
#[derive(Debug, Clone, PartialEq)]
pub enum RowScope {
    AllowAll,
    DenyAll,
    Mixed(Pred),
}

/// The person a rule is read for.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Person {
    pub address: String,
    pub department: String,
    pub affiliation: String,
    pub unit_ids: Vec<String>,
    pub position_keys: Vec<String>,
    pub group_ids: Vec<String>,
}

impl Person {
    /// The person's values for one attribute. A single-valued attribute has one value, or none when blank.
    pub fn values(&self, attr: PersonAttr) -> Vec<String> {
        let one = |s: &String| if s.is_empty() { Vec::new() } else { vec![s.clone()] };
        match attr {
            PersonAttr::Address => one(&self.address),
            PersonAttr::Department => one(&self.department),
            PersonAttr::Affiliation => one(&self.affiliation),
            PersonAttr::UnitIds => self.unit_ids.clone(),
            PersonAttr::PositionKeys => self.position_keys.clone(),
            PersonAttr::GroupIds => self.group_ids.clone(),
        }
    }

    fn is_multi(attr: PersonAttr) -> bool {
        matches!(attr, PersonAttr::UnitIds | PersonAttr::PositionKeys | PersonAttr::GroupIds)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowOp {
    Read,
    Create,
    Update,
    Delete,
}

/// A row rule: for these operations on this table, a person sees the rows where `visible_when` holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RowRule {
    pub id: String,
    pub table_id: String,
    pub ops: Vec<RowOp>,
    pub visible_when: Pred,
    /// Roles that skip this rule. A Platform Admin always skips.
    #[serde(default)]
    pub except_roles: Vec<String>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AccessError {
    #[error("a literal longer than {MAX_TEXT} characters cannot be compared")]
    LiteralTooLong,
    #[error("'{0}' is not an attribute of a person")]
    UnknownPersonAttr(String),
}

pub fn never() -> Pred {
    Pred::Not(Box::new(Pred::True))
}

fn is_never(p: &Pred) -> bool {
    matches!(p, Pred::Not(inner) if **inner == Pred::True)
}

/// The text form two sides are compared in: lowercase, the first 128 characters.
pub fn fold_text(s: &str) -> String {
    s.to_lowercase().chars().take(MAX_TEXT).collect()
}

fn texts_of(v: &Value) -> Vec<String> {
    match v {
        Value::Array(items) => items.iter().flat_map(texts_of).collect(),
        Value::String(s) if !s.is_empty() => vec![fold_text(s)],
        Value::Number(n) => vec![fold_text(&n.to_string())],
        Value::Bool(b) => vec![b.to_string()],
        _ => Vec::new(),
    }
}

/// Rejects a literal of more than 128 characters, anywhere in the predicate.
pub fn check(pred: &Pred) -> Result<(), AccessError> {
    fn long(v: &Value) -> bool {
        match v {
            Value::String(s) => s.chars().count() > MAX_TEXT,
            Value::Array(a) => a.iter().any(long),
            _ => false,
        }
    }
    match pred {
        Pred::True => Ok(()),
        Pred::Cmp { value: Operand::Literal(v), .. } if long(v) => Err(AccessError::LiteralTooLong),
        Pred::Cmp { .. } => Ok(()),
        Pred::And(ps) | Pred::Or(ps) => ps.iter().try_for_each(check),
        Pred::Not(p) => check(p),
    }
}

/// A comparison whose left side is the person. It is a constant once the person is known.
fn person_cmp(attr: PersonAttr, op: Op, operand: &Value, person: &Person) -> bool {
    let left: Vec<String> = person.values(attr).iter().map(|s| fold_text(s)).collect();
    let right = texts_of(operand);
    let any_equal = left.iter().any(|l| right.contains(l));
    match op {
        Op::Eq | Op::In => any_equal,
        Op::Ne => !any_equal,
        Op::IsEmpty => left.is_empty(),
        Op::IsNotEmpty => !left.is_empty(),
        Op::Gt | Op::Lt => false,
    }
}

fn constant(b: bool) -> Pred {
    if b {
        Pred::True
    } else {
        never()
    }
}

/// Replaces the person's values and folds constants.
pub fn substitute(pred: &Pred, person: &Person) -> Pred {
    match pred {
        Pred::True => Pred::True,
        Pred::Cmp { field, op, value } => {
            let literal = match value {
                Operand::Person(attr) => {
                    let vals = person.values(*attr);
                    if Person::is_multi(*attr) {
                        Value::Array(vals.into_iter().map(Value::String).collect())
                    } else {
                        vals.into_iter().next().map_or(Value::Null, Value::String)
                    }
                }
                Operand::Literal(v) => v.clone(),
            };
            match field.strip_prefix("person.").and_then(PersonAttr::parse) {
                Some(attr) => constant(person_cmp(attr, *op, &literal, person)),
                None => Pred::Cmp { field: field.clone(), op: *op, value: Operand::Literal(literal) },
            }
        }
        Pred::And(ps) => {
            let mut out = Vec::new();
            for p in ps.iter().map(|p| substitute(p, person)) {
                if is_never(&p) {
                    return never();
                }
                if p != Pred::True {
                    out.push(p);
                }
            }
            match out.len() {
                0 => Pred::True,
                1 => out.remove(0),
                _ => Pred::And(out),
            }
        }
        Pred::Or(ps) => {
            let mut out = Vec::new();
            for p in ps.iter().map(|p| substitute(p, person)) {
                if p == Pred::True {
                    return Pred::True;
                }
                if !is_never(&p) {
                    out.push(p);
                }
            }
            match out.len() {
                0 => never(),
                1 => out.remove(0),
                _ => Pred::Or(out),
            }
        }
        Pred::Not(p) => match substitute(p, person) {
            Pred::True => never(),
            ref n if is_never(n) => Pred::True,
            Pred::Not(inner) => *inner,
            other => Pred::Not(Box::new(other)),
        },
    }
}

impl RowScope {
    /// The scope of an already substituted predicate.
    pub fn of(pred: Pred) -> RowScope {
        if pred == Pred::True {
            RowScope::AllowAll
        } else if is_never(&pred) {
            RowScope::DenyAll
        } else {
            RowScope::Mixed(pred)
        }
    }
}

/// What `person` may reach on `table` for `op`, from the app's rules. A Platform Admin skips every
/// rule, and so does a person whose role a rule excepts. The rest combine with `And`.
pub fn row_scope(person: &Person, role: Option<&str>, platform_admin: bool, rules: &[RowRule], table: &str, op: RowOp) -> RowScope {
    if platform_admin {
        return RowScope::AllowAll;
    }
    let applying: Vec<Pred> = rules
        .iter()
        .filter(|r| r.table_id == table && r.ops.contains(&op))
        .filter(|r| !role.is_some_and(|me| r.except_roles.iter().any(|x| x.eq_ignore_ascii_case(me))))
        .map(|r| r.visible_when.clone())
        .collect();
    RowScope::of(substitute(&Pred::And(applying), person))
}
