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
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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
    /// What both scopes allow. Each is already free of person attributes, so the predicates join as they are.
    pub fn and(self, other: RowScope) -> RowScope {
        match (self, other) {
            (RowScope::DenyAll, _) | (_, RowScope::DenyAll) => RowScope::DenyAll,
            (RowScope::AllowAll, x) | (x, RowScope::AllowAll) => x,
            (RowScope::Mixed(a), RowScope::Mixed(b)) => RowScope::of(Pred::And(vec![a, b])),
        }
    }

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

// ---------------------------------------------------------------------------------------------
// Columns
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnMode {
    /// The field is removed from every response and cannot be filtered, sorted, or totalled on.
    Hidden,
    /// The field can be read and cannot be changed.
    ReadOnly,
}

/// Who a column rule does not apply to. A person matches when any list names them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Who {
    #[serde(default)]
    pub affiliations: Vec<String>,
    #[serde(default)]
    pub units: Vec<String>,
    #[serde(default)]
    pub positions: Vec<String>,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub addresses: Vec<String>,
}

impl Who {
    pub fn is_empty(&self) -> bool {
        self.affiliations.is_empty()
            && self.units.is_empty()
            && self.positions.is_empty()
            && self.roles.is_empty()
            && self.addresses.is_empty()
    }

    /// True when `person`, whose workspace role is `role`, is named.
    pub fn includes(&self, person: &Person, role: Option<&str>) -> bool {
        let same = |a: &String, b: &str| a.eq_ignore_ascii_case(b);
        self.affiliations.iter().any(|a| same(a, &person.affiliation))
            || self.units.iter().any(|u| person.unit_ids.iter().any(|p| same(u, p)))
            || self.positions.iter().any(|k| person.position_keys.iter().any(|p| same(k, p)))
            || role.is_some_and(|r| self.roles.iter().any(|x| same(x, r)))
            || self.addresses.iter().any(|a| same(a, &person.address))
    }

    /// The people who are not held back, in words: `the admin or owner role`, `faculty`.
    pub fn in_words(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        let list = |items: &[String]| match items {
            [] => String::new(),
            [one] => one.clone(),
            [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
        };
        if !self.roles.is_empty() {
            parts.push(format!("someone with the {} role", list(&self.roles)));
        }
        if !self.positions.is_empty() {
            parts.push(format!("the holder of the {} position", list(&self.positions)));
        }
        if !self.affiliations.is_empty() {
            parts.push(format!("{} staff", list(&self.affiliations)));
        }
        if !self.addresses.is_empty() {
            parts.push(list(&self.addresses.iter().take(3).cloned().collect::<Vec<_>>()));
        }
        if !self.units.is_empty() {
            parts.push("someone in a unit that is named in the rule".to_string());
        }
        match parts.as_slice() {
            [] => "an owner of this workspace".to_string(),
            all => all.join(" or "),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColumnRule {
    pub id: String,
    pub table_id: String,
    pub fields: Vec<String>,
    pub mode: ColumnMode,
    #[serde(default)]
    pub unless: Who,
    /// One plain sentence, 200 characters at most, written by the owner.
    #[serde(default)]
    pub message: Option<String>,
}

/// Why a field is held back from a person, and what they can do about it.
#[derive(Debug, Clone, PartialEq)]
pub struct Denial {
    pub field: String,
    pub rule_id: String,
    pub reason: String,
    pub remedy: String,
}

/// What a person may not see, and what they may not change, in one table.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FieldVisibility {
    pub hidden: Vec<Denial>,
    pub read_only: Vec<Denial>,
}

impl FieldVisibility {
    /// Everything either person may not see or change.
    pub fn union(mut self, other: FieldVisibility) -> FieldVisibility {
        for d in other.hidden {
            if !self.hidden.iter().any(|x| x.field == d.field) {
                self.hidden.push(d);
            }
        }
        for d in other.read_only {
            if !self.read_only.iter().any(|x| x.field == d.field) {
                self.read_only.push(d);
            }
        }
        self
    }

    pub fn hidden_names(&self) -> Vec<String> {
        self.hidden.iter().map(|d| d.field.clone()).collect()
    }
}

/// The fields of `table` the rules hold back from `person`. A Platform Admin is never held back.
pub fn field_visibility(person: &Person, role: Option<&str>, platform_admin: bool, rules: &[ColumnRule], table: &str) -> FieldVisibility {
    let mut out = FieldVisibility::default();
    if platform_admin {
        return out;
    }
    for rule in rules.iter().filter(|r| r.table_id == table && !r.unless.includes(person, role)) {
        for field in &rule.fields {
            let (verb, bucket) = match rule.mode {
                ColumnMode::Hidden => ("see", &mut out.hidden),
                ColumnMode::ReadOnly => ("change", &mut out.read_only),
            };
            if bucket.iter().any(|d| &d.field == field) {
                continue;
            }
            bucket.push(Denial {
                field: field.clone(),
                rule_id: rule.id.clone(),
                reason: rule.message.clone().unwrap_or_else(|| format!("An access rule for this app stops you from being able to {verb} '{field}'.")),
                remedy: format!("Ask {}, or an owner of this workspace.", rule.unless.in_words()),
            });
        }
    }
    out
}


impl Pred {
    pub fn node_count(&self) -> usize {
        match self {
            Pred::True | Pred::Cmp { .. } => 1,
            Pred::And(list) | Pred::Or(list) => 1 + list.iter().map(|p| p.node_count()).sum::<usize>(),
            Pred::Not(inner) => 1 + inner.node_count(),
        }
    }

    pub fn depth(&self) -> usize {
        match self {
            Pred::True | Pred::Cmp { .. } => 1,
            Pred::And(list) | Pred::Or(list) => 1 + list.iter().map(|p| p.depth()).max().unwrap_or(0),
            Pred::Not(inner) => 1 + inner.depth(),
        }
    }

    pub fn fields(&self) -> std::collections::BTreeSet<String> {
        let mut out = std::collections::BTreeSet::new();
        self.collect_fields(&mut out);
        out
    }

    fn collect_fields(&self, out: &mut std::collections::BTreeSet<String>) {
        match self {
            Pred::True => {}
            Pred::Cmp { field, .. } => {
                if !field.starts_with("person.") {
                    out.insert(field.clone());
                }
            }
            Pred::And(list) | Pred::Or(list) => {
                for p in list {
                    p.collect_fields(out);
                }
            }
            Pred::Not(inner) => {
                inner.collect_fields(out);
            }
        }
    }

    pub fn is_never_true(&self) -> bool {
        match self {
            Pred::True => false,
            Pred::Not(inner) => **inner == Pred::True || inner.is_always_true(),
            Pred::Cmp { .. } => false,
            Pred::And(ps) => {
                if ps.is_empty() {
                    return false;
                }
                if ps.iter().any(|p| p.is_never_true()) {
                    return true;
                }
                for i in 0..ps.len() {
                    for j in (i + 1)..ps.len() {
                        if let (Pred::Cmp { field: f1, op: op1, value: v1 }, Pred::Cmp { field: f2, op: op2, value: v2 }) = (&ps[i], &ps[j]) {
                            if f1 == f2 {
                                if ((*op1 == Op::Eq && *op2 == Op::Ne) || (*op1 == Op::Ne && *op2 == Op::Eq)) && v1 == v2 {
                                    return true;
                                }
                                if *op1 == Op::Eq && *op2 == Op::Eq && v1 != v2 {
                                    return true;
                                }
                                if (*op1 == Op::IsEmpty && *op2 == Op::IsNotEmpty) || (*op1 == Op::IsNotEmpty && *op2 == Op::IsEmpty) {
                                    return true;
                                }
                            }
                        }
                    }
                }
                false
            }
            Pred::Or(ps) => !ps.is_empty() && ps.iter().all(|p| p.is_never_true()),
        }
    }

    pub fn is_always_true(&self) -> bool {
        match self {
            Pred::True => true,
            Pred::Not(inner) => inner.is_never_true(),
            _ => false,
        }
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

fn plural_role(role: &str) -> &str {
    match role {
        "owner" => "owners",
        "admin" => "admins",
        "editor" => "editors",
        "viewer" => "viewers",
        "commenter" => "commenters",
        other => other,
    }
}

pub fn format_except_roles(roles: &[String]) -> String {
    let plurals: Vec<&str> = roles.iter().map(|r| plural_role(r.as_str())).collect();
    match plurals.as_slice() {
        [] => "Everyone".to_string(),
        [one] => format!("People who are not {one}"),
        [first, second] => format!("People who are not {first} or {second}"),
        more => {
            let (last, head) = more.split_last().unwrap();
            format!("People who are not {}, or {last}", head.join(", "))
        }
    }
}

pub fn format_row_ops(ops: &[RowOp]) -> String {
    let has_read = ops.contains(&RowOp::Read);
    let has_create = ops.contains(&RowOp::Create);
    let has_update = ops.contains(&RowOp::Update);
    let has_delete = ops.contains(&RowOp::Delete);

    if has_read && has_create && has_update && has_delete {
        "can see, add, change, or remove rows where".to_string()
    } else if has_read && has_update {
        "can see and change only rows where".to_string()
    } else if has_read {
        "can see only rows where".to_string()
    } else if has_create {
        "can add only rows where".to_string()
    } else if has_update {
        "can change only rows where".to_string()
    } else if has_delete {
        "can remove only rows where".to_string()
    } else {
        "can access only rows where".to_string()
    }
}

pub fn pred_in_words(pred: &Pred) -> String {
    match pred {
        Pred::True => "all rows".to_string(),
        Pred::Cmp { field, op, value } => {
            let field_label = capitalize(field);
            match (op, value) {
                (Op::Eq, Operand::Person(PersonAttr::Address)) => format!("{field_label} is themselves"),
                (Op::Eq, Operand::Person(PersonAttr::Department)) => format!("{field_label} is their department"),
                (Op::Eq, Operand::Person(PersonAttr::Affiliation)) => format!("{field_label} is their affiliation"),
                (Op::Eq, Operand::Person(PersonAttr::UnitIds)) => format!("{field_label} is in their unit"),
                (Op::Eq, Operand::Person(PersonAttr::PositionKeys)) => format!("{field_label} is in their positions"),
                (Op::Eq, Operand::Person(PersonAttr::GroupIds)) => format!("{field_label} is in their groups"),
                (Op::Ne, Operand::Person(PersonAttr::Address)) => format!("{field_label} is not themselves"),
                (Op::Ne, Operand::Person(PersonAttr::Department)) => format!("{field_label} is not their department"),
                (Op::Eq, Operand::Literal(v)) => format!("{field_label} is {v}"),
                (Op::Ne, Operand::Literal(v)) => format!("{field_label} is not {v}"),
                (Op::In, Operand::Literal(v)) => format!("{field_label} is one of {v}"),
                (Op::In, Operand::Person(attr)) => format!("{field_label} is one of their {attr:?}"),
                (Op::IsEmpty, _) => format!("{field_label} is empty"),
                (Op::IsNotEmpty, _) => format!("{field_label} is not empty"),
                (Op::Gt, Operand::Literal(v)) => format!("{field_label} is greater than {v}"),
                (Op::Gt, Operand::Person(attr)) => format!("{field_label} is greater than their {attr:?}"),
                (Op::Lt, Operand::Literal(v)) => format!("{field_label} is less than {v}"),
                (Op::Lt, Operand::Person(attr)) => format!("{field_label} is less than their {attr:?}"),
                _ => format!("{field_label} {op:?} {value:?}"),
            }
        }
        Pred::And(ps) => {
            let parts: Vec<String> = ps.iter().map(pred_in_words).collect();
            parts.join(" and ")
        }
        Pred::Or(ps) => {
            let parts: Vec<String> = ps.iter().map(pred_in_words).collect();
            parts.join(" or ")
        }
        Pred::Not(inner) => format!("not ({})", pred_in_words(inner)),
    }
}

pub fn rule_sentence(rule: &RowRule) -> String {
    let subject = format_except_roles(&rule.except_roles);
    let verb = format_row_ops(&rule.ops);
    let pred = pred_in_words(&rule.visible_when);
    format!("{subject} {verb} {pred}.")
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AccessManifest {
    #[serde(default)]
    pub rows: Vec<RowRule>,
    #[serde(default)]
    pub columns: Vec<ColumnRule>,
}
