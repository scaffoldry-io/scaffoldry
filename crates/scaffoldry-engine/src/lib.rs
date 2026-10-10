//! Scaffoldry Dynamic Engine and Manifest Renderer (Layer 3)

pub mod automation;
pub use automation::*;

use scaffoldry_core::standards::eduperson::EduPersonIdentity;
use scaffoldry_policy::ScaffoldryPolicyEngine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use thiserror::Error;
use uuid::Uuid;

#[derive(Error, Debug)]
pub enum EngineError {
    #[error("Manifest validation error: {0}")]
    ValidationError(String),

    #[error("App not found: {0}")]
    NotFound(String),

    #[error("Authorization denied by policy: {0}")]
    AccessDenied(String),

    #[error("Policy evaluation error: {0}")]
    PolicyError(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewType {
    Table,
    Grid,
    Kanban,
    Calendar,
    Gallery,
    Form,
    Dashboard,
    Detail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterOperator {
    Equals,
    NotEquals,
    Contains,
    NotContains,
    GreaterThan,
    LessThan,
    IsEmpty,
    IsNotEmpty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterConjunction {
    And,
    Or,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterClause {
    pub id: String,
    pub field_name: String,
    pub operator: FilterOperator,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompoundFilter {
    pub conjunction: FilterConjunction,
    pub clauses: Vec<FilterClause>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SortRule {
    pub id: String,
    pub field_name: String,
    pub direction: SortDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RowDensity {
    Compact,
    Medium,
    Tall,
    ExtraTall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldType {
    Text,
    Number,
    Date,
    Select,
    Boolean,
    Relation,
    // Rich Field Types
    Checkbox,
    MultiSelect,
    Currency,
    Percent,
    Rating,
    Email,
    Phone,
    Url,
    Autonumber,
    CreatedTime,
    LastModifiedTime,
    // Computed & Relational Fields
    Lookup,
    Count,
    Rollup,
    Formula,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSpec {
    pub name: String,
    pub label: String,
    pub field_type: FieldType,
    pub required: bool,
    pub ferpa_sensitive: bool,
    #[serde(default)]
    pub linked_dataset_id: Option<String>,
    #[serde(default)]
    pub linked_field: Option<String>,
    #[serde(default)]
    pub target_table_id: Option<String>,
    #[serde(default)]
    pub target_display_field: Option<String>,
    #[serde(default)]
    pub display_label_override: Option<String>,
    #[serde(default)]
    pub cardinality: Option<String>,
    #[serde(default)]
    pub allow_multiple: Option<bool>,
    #[serde(default)]
    pub link_filter: Option<serde_json::Value>,
    #[serde(default)]
    pub formula_expression: Option<String>,
    #[serde(default)]
    pub rollup_function: Option<String>,
    #[serde(default)]
    pub select_options: Vec<String>,
    #[serde(default)]
    pub currency_symbol: Option<String>,
    #[serde(default)]
    pub precision: Option<u8>,
}

/// An overlay on one field that can only add protection. It is how a compliance officer raises a
/// field's sensitivity at once, without a structural change to the manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DataLabel {
    pub ferpa_sensitive: bool,
    pub note: String,
    pub set_by: String,
}

/// Labels by `(app_slug, table_id, field)`. The table id is `""` for an untabled view. The set
/// holds exceptions only, so it stays small. It is empty until the label table exists.
pub type LabelSet = HashMap<(String, String, String), DataLabel>;

/// Whether a field is sensitive: the manifest flag, or a label that raises it. A label of
/// `false` never lowers a manifest flag. Every read of a field's sensitivity calls this.
pub fn effective_ferpa_sensitive(field: &FieldSpec, label: Option<&DataLabel>) -> bool {
    field.ferpa_sensitive || label.is_some_and(|l| l.ferpa_sensitive)
}

impl FieldSpec {
    pub fn simple(
        name: impl Into<String>,
        label: impl Into<String>,
        field_type: FieldType,
        required: bool,
        ferpa_sensitive: bool,
    ) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            field_type,
            required,
            ferpa_sensitive,
            linked_dataset_id: None,
            linked_field: None,
            target_table_id: None,
            target_display_field: None,
            display_label_override: None,
            cardinality: None,
            allow_multiple: None,
            link_filter: None,
            formula_expression: None,
            rollup_function: None,
            select_options: Vec::new(),
            currency_symbol: None,
            precision: None,
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
enum FormulaToken {
    Number(f64),
    String(String),
    Field(String),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    LParen,
    RParen,
    Comma,
}

fn tokenize_formula(input: &str) -> Option<Vec<FormulaToken>> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let n = chars.len();
    let mut i = 0;

    while i < n {
        let ch = chars[i];
        if ch.is_whitespace() {
            i += 1;
            continue;
        }

        if ch == '{' {
            let mut close = None;
            for (j, c) in chars.iter().enumerate().take(n).skip(i + 1) {
                if *c == '}' {
                    close = Some(j);
                    break;
                }
            }
            let close_idx = close?;
            let field_name: String = chars[(i + 1)..close_idx].iter().collect();
            tokens.push(FormulaToken::Field(field_name));
            i = close_idx + 1;
            continue;
        }

        if ch == '"' || ch == '\'' {
            let quote = ch;
            i += 1;
            let mut s = String::new();
            let mut closed = false;
            while i < n {
                if chars[i] == quote {
                    closed = true;
                    i += 1;
                    break;
                }
                if chars[i] == '\\' && i + 1 < n {
                    i += 1;
                    s.push(chars[i]);
                    i += 1;
                } else {
                    s.push(chars[i]);
                    i += 1;
                }
            }
            if !closed {
                return None;
            }
            tokens.push(FormulaToken::String(s));
            continue;
        }

        if ch.is_ascii_digit() || (ch == '.' && i + 1 < n && chars[i + 1].is_ascii_digit()) {
            let start = i;
            while i < n && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            let num: f64 = s.parse().ok()?;
            tokens.push(FormulaToken::Number(num));
            continue;
        }

        if ch.is_ascii_alphabetic() || ch == '_' {
            let start = i;
            while i < n && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            tokens.push(FormulaToken::Ident(s));
            continue;
        }

        if ch == '!' && i + 1 < n && chars[i + 1] == '=' {
            tokens.push(FormulaToken::NotEq);
            i += 2;
            continue;
        }
        if ch == '<' && i + 1 < n && chars[i + 1] == '=' {
            tokens.push(FormulaToken::LtEq);
            i += 2;
            continue;
        }
        if ch == '>' && i + 1 < n && chars[i + 1] == '=' {
            tokens.push(FormulaToken::GtEq);
            i += 2;
            continue;
        }
        if ch == '<' && i + 1 < n && chars[i + 1] == '>' {
            tokens.push(FormulaToken::NotEq);
            i += 2;
            continue;
        }

        match ch {
            '+' => tokens.push(FormulaToken::Plus),
            '-' => tokens.push(FormulaToken::Minus),
            '*' => tokens.push(FormulaToken::Star),
            '/' => tokens.push(FormulaToken::Slash),
            '=' => tokens.push(FormulaToken::Eq),
            '<' => tokens.push(FormulaToken::Lt),
            '>' => tokens.push(FormulaToken::Gt),
            '(' => tokens.push(FormulaToken::LParen),
            ')' => tokens.push(FormulaToken::RParen),
            ',' => tokens.push(FormulaToken::Comma),
            _ => return None,
        }
        i += 1;
    }

    Some(tokens)
}

struct FormulaParser<'a> {
    tokens: Vec<FormulaToken>,
    pos: usize,
    depth: usize,
    record: &'a Value,
}

impl<'a> FormulaParser<'a> {
    fn new(tokens: Vec<FormulaToken>, record: &'a Value) -> Self {
        Self {
            tokens,
            pos: 0,
            depth: 0,
            record,
        }
    }

    fn peek(&self) -> Option<&FormulaToken> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<FormulaToken> {
        if self.pos < self.tokens.len() {
            let tok = self.tokens[self.pos].clone();
            self.pos += 1;
            Some(tok)
        } else {
            None
        }
    }

    fn is_truthy(val: &Value) -> bool {
        match val {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
            Value::String(s) => !s.is_empty(),
            Value::Array(a) => !a.is_empty(),
            Value::Object(o) => !o.is_empty(),
        }
    }

    fn to_number(val: &Value) -> Option<f64> {
        match val {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.trim().parse::<f64>().ok(),
            _ => None,
        }
    }

    fn parse(&mut self) -> Value {
        match self.expr() {
            Ok(v) => v,
            Err(_) => Value::Null,
        }
    }

    fn check_depth(&mut self) -> Result<(), ()> {
        self.depth += 1;
        if self.depth > 32 {
            Err(())
        } else {
            Ok(())
        }
    }

    fn dec_depth(&mut self) {
        if self.depth > 0 {
            self.depth -= 1;
        }
    }

    fn expr(&mut self) -> Result<Value, ()> {
        self.or_expr()
    }

    fn or_expr(&mut self) -> Result<Value, ()> {
        self.check_depth()?;
        let mut left = self.and_expr()?;
        while let Some(FormulaToken::Ident(name)) = self.peek() {
            if name.eq_ignore_ascii_case("OR") {
                self.advance();
                let right = self.and_expr()?;
                let b = Self::is_truthy(&left) || Self::is_truthy(&right);
                left = Value::Bool(b);
            } else {
                break;
            }
        }
        self.dec_depth();
        Ok(left)
    }

    fn and_expr(&mut self) -> Result<Value, ()> {
        self.check_depth()?;
        let mut left = self.not_expr()?;
        while let Some(FormulaToken::Ident(name)) = self.peek() {
            if name.eq_ignore_ascii_case("AND") {
                self.advance();
                let right = self.not_expr()?;
                let b = Self::is_truthy(&left) && Self::is_truthy(&right);
                left = Value::Bool(b);
            } else {
                break;
            }
        }
        self.dec_depth();
        Ok(left)
    }

    fn not_expr(&mut self) -> Result<Value, ()> {
        self.check_depth()?;
        let res = if let Some(FormulaToken::Ident(name)) = self.peek() {
            if name.eq_ignore_ascii_case("NOT") {
                self.advance();
                let inner = self.not_expr()?;
                Value::Bool(!Self::is_truthy(&inner))
            } else {
                self.cmp_expr()?
            }
        } else {
            self.cmp_expr()?
        };
        self.dec_depth();
        Ok(res)
    }

    fn cmp_expr(&mut self) -> Result<Value, ()> {
        self.check_depth()?;
        let left = self.add_expr()?;
        let res = match self.peek() {
            Some(FormulaToken::Eq)
            | Some(FormulaToken::NotEq)
            | Some(FormulaToken::Lt)
            | Some(FormulaToken::LtEq)
            | Some(FormulaToken::Gt)
            | Some(FormulaToken::GtEq) => {
                let op = self.advance().unwrap();
                let right = self.add_expr()?;
                self.compare(&left, &right, &op)
            }
            _ => left,
        };
        self.dec_depth();
        Ok(res)
    }

    fn compare(&self, left: &Value, right: &Value, op: &FormulaToken) -> Value {
        let (num_l, num_r) = (Self::to_number(left), Self::to_number(right));
        if let (Some(nl), Some(nr)) = (num_l, num_r) {
            let b = match op {
                FormulaToken::Eq => nl == nr,
                FormulaToken::NotEq => nl != nr,
                FormulaToken::Lt => nl < nr,
                FormulaToken::LtEq => nl <= nr,
                FormulaToken::Gt => nl > nr,
                FormulaToken::GtEq => nl >= nr,
                _ => false,
            };
            return Value::Bool(b);
        }

        let sl = match left {
            Value::String(s) => s.as_str(),
            Value::Null => "",
            _ => "",
        };
        let sr = match right {
            Value::String(s) => s.as_str(),
            Value::Null => "",
            _ => "",
        };

        let b = match op {
            FormulaToken::Eq => left == right || sl == sr,
            FormulaToken::NotEq => left != right && sl != sr,
            FormulaToken::Lt => sl < sr,
            FormulaToken::LtEq => sl <= sr,
            FormulaToken::Gt => sl > sr,
            FormulaToken::GtEq => sl >= sr,
            _ => false,
        };
        Value::Bool(b)
    }

    fn add_expr(&mut self) -> Result<Value, ()> {
        self.check_depth()?;
        let mut left = self.mul_expr()?;
        while let Some(tok) = self.peek() {
            match tok {
                FormulaToken::Plus => {
                    self.advance();
                    let right = self.mul_expr()?;
                    if left.is_string() || right.is_string() {
                        let mut s = String::new();
                        if let Some(str_l) = left.as_str() {
                            s.push_str(str_l);
                        } else if !left.is_null() {
                            s.push_str(&left.to_string());
                        }
                        if let Some(str_r) = right.as_str() {
                            s.push_str(str_r);
                        } else if !right.is_null() {
                            s.push_str(&right.to_string());
                        }
                        left = Value::from(s);
                    } else if let (Some(nl), Some(nr)) = (Self::to_number(&left), Self::to_number(&right)) {
                        left = Value::from(nl + nr);
                    } else {
                        left = Value::Null;
                    }
                }
                FormulaToken::Minus => {
                    self.advance();
                    let right = self.mul_expr()?;
                    if let (Some(nl), Some(nr)) = (Self::to_number(&left), Self::to_number(&right)) {
                        left = Value::from(nl - nr);
                    } else {
                        left = Value::Null;
                    }
                }
                _ => break,
            }
        }
        self.dec_depth();
        Ok(left)
    }

    fn mul_expr(&mut self) -> Result<Value, ()> {
        self.check_depth()?;
        let mut left = self.unary_expr()?;
        while let Some(tok) = self.peek() {
            match tok {
                FormulaToken::Star => {
                    self.advance();
                    let right = self.unary_expr()?;
                    if let (Some(nl), Some(nr)) = (Self::to_number(&left), Self::to_number(&right)) {
                        left = Value::from(nl * nr);
                    } else {
                        left = Value::Null;
                    }
                }
                FormulaToken::Slash => {
                    self.advance();
                    let right = self.unary_expr()?;
                    if let (Some(nl), Some(nr)) = (Self::to_number(&left), Self::to_number(&right)) {
                        if nr == 0.0 {
                            left = Value::Null;
                        } else {
                            left = Value::from(nl / nr);
                        }
                    } else {
                        left = Value::Null;
                    }
                }
                _ => break,
            }
        }
        self.dec_depth();
        Ok(left)
    }

    fn unary_expr(&mut self) -> Result<Value, ()> {
        self.check_depth()?;
        let res = match self.peek() {
            Some(FormulaToken::Minus) => {
                self.advance();
                let inner = self.unary_expr()?;
                if let Some(n) = Self::to_number(&inner) {
                    Value::from(-n)
                } else {
                    Value::Null
                }
            }
            _ => self.primary_expr()?,
        };
        self.dec_depth();
        Ok(res)
    }

    fn primary_expr(&mut self) -> Result<Value, ()> {
        self.check_depth()?;
        let res = match self.advance() {
            Some(FormulaToken::Number(n)) => Value::from(n),
            Some(FormulaToken::String(s)) => Value::from(s),
            Some(FormulaToken::Field(field)) => {
                self.record.get(&field).cloned().unwrap_or(Value::Null)
            }
            Some(FormulaToken::LParen) => {
                let inner = self.expr()?;
                if let Some(FormulaToken::RParen) = self.advance() {
                    inner
                } else {
                    return Err(());
                }
            }
            Some(FormulaToken::Ident(name)) => {
                if let Some(FormulaToken::LParen) = self.peek() {
                    self.advance();
                    let mut args = Vec::new();
                    if let Some(FormulaToken::RParen) = self.peek() {
                        self.advance();
                    } else {
                        args.push(self.expr()?);
                        while let Some(FormulaToken::Comma) = self.peek() {
                            self.advance();
                            args.push(self.expr()?);
                        }
                        if let Some(FormulaToken::RParen) = self.advance() {
                        } else {
                            return Err(());
                        }
                    }
                    self.call_func(&name.to_ascii_uppercase(), &args)
                } else {
                    Value::Null
                }
            }
            _ => Value::Null,
        };
        self.dec_depth();
        Ok(res)
    }

    fn call_func(&self, name: &str, args: &[Value]) -> Value {
        match name {
            "IF" => {
                let cond = args.first().unwrap_or(&Value::Null);
                if Self::is_truthy(cond) {
                    args.get(1).cloned().unwrap_or(Value::Null)
                } else {
                    args.get(2).cloned().unwrap_or(Value::Null)
                }
            }
            "AND" => {
                if args.is_empty() {
                    Value::Null
                } else {
                    Value::Bool(args.iter().all(Self::is_truthy))
                }
            }
            "OR" => {
                if args.is_empty() {
                    Value::Null
                } else {
                    Value::Bool(args.iter().any(Self::is_truthy))
                }
            }
            "NOT" => {
                if args.is_empty() {
                    Value::Null
                } else {
                    Value::Bool(!Self::is_truthy(&args[0]))
                }
            }
            "ROUND" => {
                if let Some(n) = args.first().and_then(Self::to_number) {
                    let digits = args.get(1).and_then(Self::to_number).unwrap_or(0.0) as i32;
                    let factor = 10f64.powi(digits);
                    let rounded = (n * factor).round() / factor;
                    Value::from(rounded)
                } else {
                    Value::Null
                }
            }
            "ABS" => {
                if let Some(n) = args.first().and_then(Self::to_number) {
                    Value::from(n.abs())
                } else {
                    Value::Null
                }
            }
            "CONCAT" => {
                let mut s = String::new();
                for a in args {
                    if !a.is_null() {
                        if let Some(str_val) = a.as_str() {
                            s.push_str(str_val);
                        } else if let Some(n) = a.as_f64() {
                            s.push_str(&n.to_string());
                        } else if let Some(b) = a.as_bool() {
                            s.push_str(&b.to_string());
                        }
                    }
                }
                Value::from(s)
            }
            "LEN" => {
                let val = args.first().unwrap_or(&Value::Null);
                if val.is_null() {
                    Value::from(0)
                } else if let Some(s) = val.as_str() {
                    Value::from(s.len())
                } else {
                    Value::from(val.to_string().len())
                }
            }
            "BLANK" => Value::Null,
            "ISBLANK" => {
                let val = args.first().unwrap_or(&Value::Null);
                let is_blank = val.is_null() || (val.as_str().map(|s| s.is_empty()).unwrap_or(false));
                Value::Bool(is_blank)
            }
            _ => Value::Null,
        }
    }
}

pub fn evaluate_formula(expression: &str, record: &Value) -> Value {
    let expr = expression.trim();
    if expr.is_empty() || expr.len() > 500 {
        return Value::Null;
    }

    if let Some(tokens) = tokenize_formula(expr) {
        let mut parser = FormulaParser::new(tokens, record);
        parser.parse()
    } else {
        Value::Null
    }
}

pub fn parse_operand(op: &str, record: &Value) -> Value {
    evaluate_formula(op, record)
}

pub fn compute_field_value(
    field: &FieldSpec,
    record: &Value,
    linked_records: &[Value],
) -> Value {
    match field.field_type {
        FieldType::Formula => {
            if let Some(expr) = &field.formula_expression {
                evaluate_formula(expr, record)
            } else {
                Value::Null
            }
        }
        FieldType::Lookup => {
            if let Some(target_col) = &field.target_display_field {
                let values: Vec<Value> = linked_records
                    .iter()
                    .filter_map(|r| r.get(target_col).cloned())
                    .collect();
                if values.len() == 1 {
                    values[0].clone()
                } else {
                    Value::Array(values)
                }
            } else {
                Value::Null
            }
        }
        FieldType::Count => Value::from(linked_records.len()),
        FieldType::Rollup => {
            let target_col = field.target_display_field.as_deref().unwrap_or("amount");
            let numbers: Vec<f64> = linked_records
                .iter()
                .filter_map(|r| {
                    r.get(target_col)
                        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
                })
                .collect();
            let func = field.rollup_function.as_deref().unwrap_or("sum");
            match func.to_lowercase().as_str() {
                "sum" => Value::from(numbers.iter().sum::<f64>()),
                "avg" => {
                    if numbers.is_empty() {
                        Value::from(0.0)
                    } else {
                        Value::from(numbers.iter().sum::<f64>() / numbers.len() as f64)
                    }
                }
                "min" => {
                    let min = numbers.iter().cloned().fold(f64::INFINITY, f64::min);
                    if min.is_infinite() {
                        Value::Null
                    } else {
                        Value::from(min)
                    }
                }
                "max" => {
                    let max = numbers.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                    if max.is_infinite() {
                        Value::Null
                    } else {
                        Value::from(max)
                    }
                }
                "count" => Value::from(numbers.len()),
                _ => Value::Null,
            }
        }
        _ => record.get(&field.name).cloned().unwrap_or(Value::Null),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableRelationship {
    pub id: String,
    pub name: String,
    pub source_table_id: String,
    pub target_table_id: String,
    pub source_field: String,
    pub target_field: String,
    pub relationship_type: String,
    pub display_field: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppTable {
    pub id: String,
    pub name: String,
    pub slug: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    pub fields: Vec<FieldSpec>,
    #[serde(default)]
    pub primary_field: Option<String>,
    #[serde(default)]
    pub sample_records: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppView {
    pub id: String,
    #[serde(default)]
    pub table_id: Option<String>,
    pub title: String,
    pub view_type: ViewType,
    #[serde(default)]
    pub fields: Vec<FieldSpec>,
    #[serde(default)]
    pub filters: Option<CompoundFilter>,
    #[serde(default)]
    pub sort_rules: Vec<SortRule>,
    #[serde(default)]
    pub group_by_field: Option<String>,
    #[serde(default)]
    pub row_density: Option<RowDensity>,
    #[serde(default)]
    pub kanban_column_field: Option<String>,
    #[serde(default)]
    pub calendar_date_field: Option<String>,
    #[serde(default)]
    pub column_order: Vec<String>,
    #[serde(default)]
    pub column_widths: Vec<(String, u32)>,
    #[serde(default)]
    pub hidden_columns: Vec<String>,
    #[serde(default)]
    pub frozen_through: Option<String>,
    #[serde(default)]
    pub column_summary: Vec<(String, String)>,
}

impl AppView {
    pub fn table(
        id: impl Into<String>,
        title: impl Into<String>,
        view_type: ViewType,
        fields: Vec<FieldSpec>,
    ) -> Self {
        Self {
            id: id.into(),
            table_id: None,
            title: title.into(),
            view_type,
            fields,
            filters: None,
            sort_rules: Vec::new(),
            group_by_field: None,
            row_density: None,
            kanban_column_field: None,
            calendar_date_field: None,
            column_order: Vec::new(),
            column_widths: Vec::new(),
            hidden_columns: Vec::new(),
            frozen_through: None,
            column_summary: Vec::new(),
        }
    }
}

pub fn matches_filter(record: &Value, filter: &CompoundFilter) -> bool {
    if filter.clauses.is_empty() {
        return true;
    }
    match filter.conjunction {
        FilterConjunction::And => filter.clauses.iter().all(|c| matches_clause(record, c)),
        FilterConjunction::Or => filter.clauses.iter().any(|c| matches_clause(record, c)),
    }
}

pub fn matches_clause(record: &Value, clause: &FilterClause) -> bool {
    let cell_val = record.get(&clause.field_name);
    if clause.operator != FilterOperator::IsEmpty
        && clause.operator != FilterOperator::IsNotEmpty
        && clause.value.trim().is_empty()
    {
        return true;
    }
    match clause.operator {
        FilterOperator::IsEmpty => {
            cell_val.is_none()
                || cell_val == Some(&Value::Null)
                || cell_val == Some(&Value::String(String::new()))
        }
        FilterOperator::IsNotEmpty => {
            cell_val.is_some()
                && cell_val != Some(&Value::Null)
                && cell_val != Some(&Value::String(String::new()))
        }
        FilterOperator::Equals => {
            let s = cell_val
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    _ => String::new(),
                })
                .unwrap_or_default();
            s.eq_ignore_ascii_case(&clause.value)
        }
        FilterOperator::NotEquals => {
            let s = cell_val
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    _ => String::new(),
                })
                .unwrap_or_default();
            !s.eq_ignore_ascii_case(&clause.value)
        }
        FilterOperator::Contains => {
            let s = cell_val
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    _ => String::new(),
                })
                .unwrap_or_default();
            s.to_lowercase().contains(&clause.value.to_lowercase())
        }
        FilterOperator::NotContains => {
            let s = cell_val
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    _ => String::new(),
                })
                .unwrap_or_default();
            !s.to_lowercase().contains(&clause.value.to_lowercase())
        }
        FilterOperator::GreaterThan => {
            let n1 = cell_val.and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)));
            let n2 = clause.value.parse::<f64>().ok();
            match (n1, n2) {
                (Some(a), Some(b)) => a > b,
                _ => false,
            }
        }
        FilterOperator::LessThan => {
            let n1 = cell_val.and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)));
            let n2 = clause.value.parse::<f64>().ok();
            match (n1, n2) {
                (Some(a), Some(b)) => a < b,
                _ => false,
            }
        }
    }
}

pub fn sort_records(records: &mut [Value], rules: &[SortRule]) {
    records.sort_by(|a, b| {
        for rule in rules {
            let va = a.get(&rule.field_name);
            let vb = b.get(&rule.field_name);
            let ord = compare_values(va, vb);
            let directed = match rule.direction {
                SortDirection::Asc => ord,
                SortDirection::Desc => ord.reverse(),
            };
            if !directed.is_eq() {
                return directed;
            }
        }
        std::cmp::Ordering::Equal
    });
}

fn compare_values(a: Option<&Value>, b: Option<&Value>) -> std::cmp::Ordering {
    match (a, b) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(va), Some(vb)) => {
            if let (Some(na), Some(nb)) = (va.as_f64(), vb.as_f64()) {
                na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
            } else if let (Some(sa), Some(sb)) = (va.as_str(), vb.as_str()) {
                sa.cmp(sb)
            } else {
                std::cmp::Ordering::Equal
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppManifest {
    pub slug: String,
    pub title: String,
    pub description: String,
    pub organization_code: String,
    pub department: String,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub herm_capability_id: Option<String>,
    #[serde(default)]
    pub custom_domain: Option<String>,
    #[serde(default)]
    pub custom_domain_verified: bool,
    #[serde(default)]
    pub tables: Vec<AppTable>,
    #[serde(default)]
    pub relationships: Vec<TableRelationship>,
    pub views: Vec<AppView>,
    #[serde(default)]
    pub ceds_mappings: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmittedRecord {
    pub id: Uuid,
    pub app_slug: String,
    pub data: Value,
    pub ceds_mapping: Value,
    pub is_ferpa_sensitive: bool,
}

pub trait HostRouter {
    fn resolve_by_host(&self, host: &str) -> Option<&AppManifest>;
    fn resolve_by_slug(&self, slug: &str) -> Option<&AppManifest>;
}

pub struct ManifestEngine {
    manifests_by_slug: HashMap<String, AppManifest>,
    manifests_by_domain: HashMap<String, String>,
}

impl ManifestEngine {
    pub fn new() -> Result<Self, EngineError> {
        // Fail construction early if the built in policy set does not load.
        ScaffoldryPolicyEngine::default_institutional_engine()
            .map_err(|e| EngineError::PolicyError(e.to_string()))?;
        Ok(Self {
            manifests_by_slug: HashMap::new(),
            manifests_by_domain: HashMap::new(),
        })
    }

    pub fn manifests_count(&self) -> usize {
        self.manifests_by_slug.len()
    }

    pub fn list_manifests(&self) -> Vec<AppManifest> {
        self.manifests_by_slug.values().cloned().collect()
    }

    pub fn register_manifest(&mut self, manifest: AppManifest) -> Result<(), EngineError> {
        if manifest.slug.trim().is_empty() {
            return Err(EngineError::ValidationError("App slug cannot be empty".to_string()));
        }

        if let Some(domain) = &manifest.custom_domain {
            if manifest.custom_domain_verified {
                self.manifests_by_domain.insert(domain.clone(), manifest.slug.clone());
            }
        }

        self.manifests_by_slug.insert(manifest.slug.clone(), manifest);
        Ok(())
    }

    pub fn submit_record(
        &self,
        caller: &EduPersonIdentity,
        app_slug: &str,
        payload: &Value,
    ) -> Result<SubmittedRecord, EngineError> {
        self.submit_record_labelled(caller, app_slug, payload, &LabelSet::new())
    }

    /// Submits a record, deciding sensitivity with the given labels.
    pub fn submit_record_labelled(
        &self,
        caller: &EduPersonIdentity,
        app_slug: &str,
        payload: &Value,
        labels: &LabelSet,
    ) -> Result<SubmittedRecord, EngineError> {
        let manifest = self
            .manifests_by_slug
            .get(app_slug)
            .ok_or_else(|| EngineError::NotFound(format!("App '{}' not found", app_slug)))?;

        let _ = caller;

        let mut is_ferpa_sensitive = false;
        let mut applied_ceds = HashMap::new();

        let record_table_id = payload.get("_table_id").and_then(|v| v.as_str());

        for view in &manifest.views {
            if let (Some(rec_tid), Some(view_tid)) = (record_table_id, view.table_id.as_deref()) {
                if rec_tid != view_tid {
                    continue;
                }
            }
            for field in &view.fields {
                let val = payload.get(&field.name);
                if field.required && val.is_none_or(|v| v.is_null()) {
                    return Err(EngineError::ValidationError(format!(
                        "Required field '{}' is missing in payload",
                        field.name
                    )));
                }

                let key = (
                    app_slug.to_string(),
                    view.table_id.clone().unwrap_or_default(),
                    field.name.clone(),
                );
                if val.is_some() && effective_ferpa_sensitive(field, labels.get(&key)) {
                    is_ferpa_sensitive = true;
                }

                if let Some(ceds_code) = manifest.ceds_mappings.get(&field.name) {
                    applied_ceds.insert(field.name.clone(), ceds_code.clone());
                }
            }
        }

        Ok(SubmittedRecord {
            id: Uuid::new_v4(),
            app_slug: app_slug.to_string(),
            data: payload.clone(),
            ceds_mapping: serde_json::to_value(applied_ceds).unwrap_or_default(),
            is_ferpa_sensitive,
        })
    }
}

impl HostRouter for ManifestEngine {
    fn resolve_by_host(&self, host: &str) -> Option<&AppManifest> {
        let clean_host = host.split(':').next().unwrap_or(host);
        let slug = self.manifests_by_domain.get(clean_host)?;
        self.manifests_by_slug.get(slug)
    }

    fn resolve_by_slug(&self, slug: &str) -> Option<&AppManifest> {
        self.manifests_by_slug.get(slug)
    }
}

/// Formats a single cell value for RFC 4180 CSV export.
pub fn escape_csv_cell(cell: &str) -> String {
    if cell.contains(',') || cell.contains('"') || cell.contains('\n') || cell.contains('\r') {
        let escaped = cell.replace('"', "\"\"");
        format!("\"{}\"", escaped)
    } else {
        cell.to_string()
    }
}

/// Generates an RFC 4180 CSV string from a slice of header names and JSON records.
pub fn export_records_to_csv(headers: &[&str], records: &[Value]) -> String {
    let mut out = String::new();

    let header_line: Vec<String> = headers.iter().map(|h| escape_csv_cell(h)).collect();
    out.push_str(&header_line.join(","));
    out.push('\n');

    for record in records {
        let row: Vec<String> = headers
            .iter()
            .map(|&h| match record.get(h) {
                Some(Value::String(s)) => escape_csv_cell(s),
                Some(Value::Number(n)) => n.to_string(),
                Some(Value::Bool(b)) => b.to_string(),
                Some(Value::Array(arr)) => {
                    let items: Vec<String> = arr
                        .iter()
                        .filter_map(|v| match v {
                            Value::String(s) => Some(s.clone()),
                            Value::Number(n) => Some(n.to_string()),
                            _ => None,
                        })
                        .collect();
                    escape_csv_cell(&items.join(", "))
                }
                Some(Value::Null) | None => String::new(),
                Some(other) => escape_csv_cell(&other.to_string()),
            })
            .collect();
        out.push_str(&row.join(","));
        out.push('\n');
    }

    out
}

/// Parses an RFC 4180 CSV text into a vector of JSON objects.
pub fn parse_csv_to_records(csv_text: &str) -> Result<Vec<Value>, String> {
    let trimmed = csv_text.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut current_row: Vec<String> = Vec::new();
    let mut current_field = String::new();
    let mut in_quotes = false;
    let mut chars = trimmed.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    current_field.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                current_field.push(ch);
            }
        } else {
            match ch {
                '"' => {
                    in_quotes = true;
                }
                ',' => {
                    current_row.push(current_field.trim().to_string());
                    current_field.clear();
                }
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    current_row.push(current_field.trim().to_string());
                    current_field.clear();
                    rows.push(std::mem::take(&mut current_row));
                }
                '\n' => {
                    current_row.push(current_field.trim().to_string());
                    current_field.clear();
                    rows.push(std::mem::take(&mut current_row));
                }
                _ => {
                    current_field.push(ch);
                }
            }
        }
    }

    if !current_field.is_empty() || !current_row.is_empty() {
        current_row.push(current_field.trim().to_string());
        rows.push(current_row);
    }

    if rows.is_empty() {
        return Ok(Vec::new());
    }

    let headers = rows[0].clone();
    let mut records = Vec::new();

    for row in rows.into_iter().skip(1) {
        if row.is_empty() || (row.len() == 1 && row[0].is_empty()) {
            continue;
        }
        let mut map = serde_json::Map::new();
        for (i, header) in headers.iter().enumerate() {
            if header.is_empty() {
                continue;
            }
            let cell = row.get(i).map(|s| s.as_str()).unwrap_or("");
            if let Ok(num) = cell.parse::<f64>() {
                map.insert(header.clone(), serde_json::json!(num));
            } else if cell.eq_ignore_ascii_case("true") {
                map.insert(header.clone(), serde_json::json!(true));
            } else if cell.eq_ignore_ascii_case("false") {
                map.insert(header.clone(), serde_json::json!(false));
            } else {
                map.insert(header.clone(), serde_json::json!(cell));
            }
        }
        records.push(Value::Object(map));
    }

    Ok(records)
}

/// Duplicates a record, giving it a new ID and appending "(Copy)" to the primary field.
pub fn duplicate_record(record: &Value, primary_field: &str, new_id: &str) -> Value {
    let mut cloned = record.clone();
    if let Value::Object(ref mut map) = cloned {
        map.insert("id".to_string(), Value::String(new_id.to_string()));
        if let Some(Value::String(val)) = map.get(primary_field) {
            map.insert(primary_field.to_string(), Value::String(format!("{} (Copy)", val)));
        }
    }
    cloned
}

/// Batch removes records whose "id" matches any id in `ids_to_delete`.
pub fn batch_delete_records(records: &mut Vec<Value>, ids_to_delete: &[&str]) {
    records.retain(|r| {
        if let Some(id_val) = r.get("id").and_then(|v| v.as_str()) {
            !ids_to_delete.contains(&id_val)
        } else {
            true
        }
    });
}

