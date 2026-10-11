//! Sovereign Formula Parser and Evaluator (Rust Only)

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_FORMULA_CHARS: usize = 500;
pub const MAX_FORMULA_NODES: usize = 200;
pub const MAX_RESULT_TEXT_CHARS: usize = 100_000;

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct FormulaError {
    pub message: String,
    pub position: usize,
}

#[derive(Debug, Clone)]
pub struct FormulaInfo {
    pub node_count: usize,
    pub field_refs: Vec<String>,
}

#[derive(Debug, PartialEq, Clone)]
pub enum FormulaToken {
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

#[derive(Debug, PartialEq, Clone)]
pub struct TokenSpan {
    pub token: FormulaToken,
    pub start: usize,
    pub end: usize,
}

pub fn tokenize_formula(input: &str) -> Result<Vec<TokenSpan>, FormulaError> {
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

        let start = i;

        if ch == '{' {
            let mut close = None;
            for (j, c) in chars.iter().enumerate().take(n).skip(i + 1) {
                if *c == '}' {
                    close = Some(j);
                    break;
                }
            }
            let close_idx = match close {
                Some(idx) => idx,
                None => {
                    return Err(FormulaError {
                        message: "Unclosed field reference: missing '}'".to_string(),
                        position: start,
                    });
                }
            };
            let field_name: String = chars[(i + 1)..close_idx].iter().collect();
            tokens.push(TokenSpan {
                token: FormulaToken::Field(field_name),
                start,
                end: close_idx + 1,
            });
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
                return Err(FormulaError {
                    message: "Unclosed string literal".to_string(),
                    position: start,
                });
            }
            tokens.push(TokenSpan {
                token: FormulaToken::String(s),
                start,
                end: i,
            });
            continue;
        }

        if ch.is_ascii_digit() || (ch == '.' && i + 1 < n && chars[i + 1].is_ascii_digit()) {
            let num_start = i;
            while i < n && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let s: String = chars[num_start..i].iter().collect();
            let num: f64 = match s.parse() {
                Ok(v) => v,
                Err(_) => {
                    return Err(FormulaError {
                        message: format!("Invalid numeric literal: '{s}'"),
                        position: num_start,
                    });
                }
            };
            tokens.push(TokenSpan {
                token: FormulaToken::Number(num),
                start: num_start,
                end: i,
            });
            continue;
        }

        if ch.is_ascii_alphabetic() || ch == '_' {
            let ident_start = i;
            while i < n && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let s: String = chars[ident_start..i].iter().collect();
            tokens.push(TokenSpan {
                token: FormulaToken::Ident(s),
                start: ident_start,
                end: i,
            });
            continue;
        }

        if ch == '!' && i + 1 < n && chars[i + 1] == '=' {
            tokens.push(TokenSpan {
                token: FormulaToken::NotEq,
                start,
                end: i + 2,
            });
            i += 2;
            continue;
        }
        if ch == '<' && i + 1 < n && chars[i + 1] == '=' {
            tokens.push(TokenSpan {
                token: FormulaToken::LtEq,
                start,
                end: i + 2,
            });
            i += 2;
            continue;
        }
        if ch == '>' && i + 1 < n && chars[i + 1] == '=' {
            tokens.push(TokenSpan {
                token: FormulaToken::GtEq,
                start,
                end: i + 2,
            });
            i += 2;
            continue;
        }
        if ch == '<' && i + 1 < n && chars[i + 1] == '>' {
            tokens.push(TokenSpan {
                token: FormulaToken::NotEq,
                start,
                end: i + 2,
            });
            i += 2;
            continue;
        }

        let tok = match ch {
            '+' => FormulaToken::Plus,
            '-' => FormulaToken::Minus,
            '*' => FormulaToken::Star,
            '/' => FormulaToken::Slash,
            '=' => FormulaToken::Eq,
            '<' => FormulaToken::Lt,
            '>' => FormulaToken::Gt,
            '(' => FormulaToken::LParen,
            ')' => FormulaToken::RParen,
            ',' => FormulaToken::Comma,
            _ => {
                return Err(FormulaError {
                    message: format!("Unexpected character: '{ch}'"),
                    position: start,
                });
            }
        };
        tokens.push(TokenSpan {
            token: tok,
            start,
            end: i + 1,
        });
        i += 1;
    }

    Ok(tokens)
}

#[derive(Debug, Clone)]
pub enum AstNode {
    Number(f64),
    String(String),
    Field(String),
    UnaryMinus(Box<AstNode>),
    BinaryOp {
        op: FormulaToken,
        left: Box<AstNode>,
        right: Box<AstNode>,
    },
    Call {
        name: String,
        args: Vec<AstNode>,
    },
}

impl AstNode {
    pub fn count_nodes(&self) -> usize {
        match self {
            AstNode::Number(_) | AstNode::String(_) | AstNode::Field(_) => 1,
            AstNode::UnaryMinus(inner) => 1 + inner.count_nodes(),
            AstNode::BinaryOp { left, right, .. } => 1 + left.count_nodes() + right.count_nodes(),
            AstNode::Call { args, .. } => 1 + args.iter().map(|a| a.count_nodes()).sum::<usize>(),
        }
    }

    pub fn collect_field_refs(&self, refs: &mut HashSet<String>) {
        match self {
            AstNode::Field(name) => {
                refs.insert(name.clone());
            }
            AstNode::UnaryMinus(inner) => inner.collect_field_refs(refs),
            AstNode::BinaryOp { left, right, .. } => {
                left.collect_field_refs(refs);
                right.collect_field_refs(refs);
            }
            AstNode::Call { args, .. } => {
                for a in args {
                    a.collect_field_refs(refs);
                }
            }
            _ => {}
        }
    }
}

pub struct AstParser {
    tokens: Vec<TokenSpan>,
    pos: usize,
    depth: usize,
    total_nodes: usize,
}

impl AstParser {
    pub fn new(tokens: Vec<TokenSpan>) -> Self {
        Self {
            tokens,
            pos: 0,
            depth: 0,
            total_nodes: 0,
        }
    }

    fn peek(&self) -> Option<&TokenSpan> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<TokenSpan> {
        if self.pos < self.tokens.len() {
            let tok = self.tokens[self.pos].clone();
            self.pos += 1;
            Some(tok)
        } else {
            None
        }
    }

    fn check_limits(&mut self) -> Result<(), FormulaError> {
        self.depth += 1;
        self.total_nodes += 1;
        if self.total_nodes > MAX_FORMULA_NODES {
            let pos = self.peek().map(|t| t.start).unwrap_or(0);
            return Err(FormulaError {
                message: format!("Formula exceeds maximum node limit of {MAX_FORMULA_NODES}"),
                position: pos,
            });
        }
        if self.depth > 32 {
            let pos = self.peek().map(|t| t.start).unwrap_or(0);
            return Err(FormulaError {
                message: "Formula exceeds maximum nesting depth of 32".to_string(),
                position: pos,
            });
        }
        Ok(())
    }

    fn dec_depth(&mut self) {
        if self.depth > 0 {
            self.depth -= 1;
        }
    }

    pub fn parse_all(&mut self) -> Result<AstNode, FormulaError> {
        if self.tokens.is_empty() {
            return Err(FormulaError {
                message: "Formula expression is empty".to_string(),
                position: 0,
            });
        }
        let ast = self.expr()?;
        if let Some(tok) = self.peek() {
            return Err(FormulaError {
                message: format!("Unexpected token after expression: {:?}", tok.token),
                position: tok.start,
            });
        }
        Ok(ast)
    }

    fn expr(&mut self) -> Result<AstNode, FormulaError> {
        self.or_expr()
    }

    fn or_expr(&mut self) -> Result<AstNode, FormulaError> {
        self.check_limits()?;
        let mut left = self.and_expr()?;
        while let Some(span) = self.peek().cloned() {
            if let FormulaToken::Ident(name) = &span.token {
                if name.eq_ignore_ascii_case("OR") {
                    self.advance();
                    let right = self.and_expr()?;
                    left = AstNode::BinaryOp {
                        op: FormulaToken::Ident("OR".to_string()),
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                    continue;
                }
            }
            break;
        }
        self.dec_depth();
        Ok(left)
    }

    fn and_expr(&mut self) -> Result<AstNode, FormulaError> {
        self.check_limits()?;
        let mut left = self.not_expr()?;
        while let Some(span) = self.peek().cloned() {
            if let FormulaToken::Ident(name) = &span.token {
                if name.eq_ignore_ascii_case("AND") {
                    self.advance();
                    let right = self.not_expr()?;
                    left = AstNode::BinaryOp {
                        op: FormulaToken::Ident("AND".to_string()),
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                    continue;
                }
            }
            break;
        }
        self.dec_depth();
        Ok(left)
    }

    fn not_expr(&mut self) -> Result<AstNode, FormulaError> {
        self.check_limits()?;
        if let Some(span) = self.peek().cloned() {
            if let FormulaToken::Ident(name) = &span.token {
                if name.eq_ignore_ascii_case("NOT") {
                    self.advance();
                    let inner = self.not_expr()?;
                    self.dec_depth();
                    return Ok(AstNode::Call {
                        name: "NOT".to_string(),
                        args: vec![inner],
                    });
                }
            }
        }
        let res = self.cmp_expr()?;
        self.dec_depth();
        Ok(res)
    }

    fn cmp_expr(&mut self) -> Result<AstNode, FormulaError> {
        self.check_limits()?;
        let left = self.add_expr()?;
        let res = if let Some(span) = self.peek().cloned() {
            match span.token {
                FormulaToken::Eq
                | FormulaToken::NotEq
                | FormulaToken::Lt
                | FormulaToken::LtEq
                | FormulaToken::Gt
                | FormulaToken::GtEq => {
                    self.advance();
                    let right = self.add_expr()?;
                    AstNode::BinaryOp {
                        op: span.token,
                        left: Box::new(left),
                        right: Box::new(right),
                    }
                }
                _ => left,
            }
        } else {
            left
        };
        self.dec_depth();
        Ok(res)
    }

    fn add_expr(&mut self) -> Result<AstNode, FormulaError> {
        self.check_limits()?;
        let mut left = self.mul_expr()?;
        while let Some(span) = self.peek().cloned() {
            match span.token {
                FormulaToken::Plus | FormulaToken::Minus => {
                    self.advance();
                    let right = self.mul_expr()?;
                    left = AstNode::BinaryOp {
                        op: span.token,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                _ => break,
            }
        }
        self.dec_depth();
        Ok(left)
    }

    fn mul_expr(&mut self) -> Result<AstNode, FormulaError> {
        self.check_limits()?;
        let mut left = self.unary_expr()?;
        while let Some(span) = self.peek().cloned() {
            match span.token {
                FormulaToken::Star | FormulaToken::Slash => {
                    self.advance();
                    let right = self.unary_expr()?;
                    left = AstNode::BinaryOp {
                        op: span.token,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                _ => break,
            }
        }
        self.dec_depth();
        Ok(left)
    }

    fn unary_expr(&mut self) -> Result<AstNode, FormulaError> {
        self.check_limits()?;
        if let Some(span) = self.peek().cloned() {
            if span.token == FormulaToken::Minus {
                self.advance();
                let inner = self.unary_expr()?;
                self.dec_depth();
                return Ok(AstNode::UnaryMinus(Box::new(inner)));
            }
        }
        let res = self.primary_expr()?;
        self.dec_depth();
        Ok(res)
    }

    fn primary_expr(&mut self) -> Result<AstNode, FormulaError> {
        self.check_limits()?;
        let span = match self.advance() {
            Some(s) => s,
            None => {
                let pos = self.tokens.last().map(|t| t.end).unwrap_or(0);
                return Err(FormulaError {
                    message: "Unexpected end of formula".to_string(),
                    position: pos,
                });
            }
        };

        let res = match span.token {
            FormulaToken::Number(n) => AstNode::Number(n),
            FormulaToken::String(s) => AstNode::String(s),
            FormulaToken::Field(name) => AstNode::Field(name),
            FormulaToken::LParen => {
                let inner = self.expr()?;
                match self.advance() {
                    Some(close) if close.token == FormulaToken::RParen => inner,
                    Some(other) => {
                        return Err(FormulaError {
                            message: "Expected closing parenthesis ')'".to_string(),
                            position: other.start,
                        });
                    }
                    None => {
                        return Err(FormulaError {
                            message: "Missing closing parenthesis ')'".to_string(),
                            position: span.end,
                        });
                    }
                }
            }
            FormulaToken::Ident(name) => {
                if let Some(next) = self.peek() {
                    if next.token == FormulaToken::LParen {
                        self.advance();
                        let mut args = Vec::new();
                        if let Some(close) = self.peek() {
                            if close.token == FormulaToken::RParen {
                                self.advance();
                                self.dec_depth();
                                return Ok(AstNode::Call {
                                    name: name.to_ascii_uppercase(),
                                    args,
                                });
                            }
                        }
                        args.push(self.expr()?);
                        while let Some(c) = self.peek() {
                            if c.token == FormulaToken::Comma {
                                self.advance();
                                args.push(self.expr()?);
                            } else {
                                break;
                            }
                        }
                        match self.advance() {
                            Some(close) if close.token == FormulaToken::RParen => {}
                            Some(other) => {
                                return Err(FormulaError {
                                    message: "Expected closing parenthesis ')' for function call".to_string(),
                                    position: other.start,
                                });
                            }
                            None => {
                                return Err(FormulaError {
                                    message: "Unclosed function argument list".to_string(),
                                    position: span.end,
                                });
                            }
                        }
                        AstNode::Call {
                            name: name.to_ascii_uppercase(),
                            args,
                        }
                    } else {
                        return Err(FormulaError {
                            message: format!("Unexpected identifier '{name}': did you mean to reference '{{{name}}}'?"),
                            position: span.start,
                        });
                    }
                } else {
                    return Err(FormulaError {
                        message: format!("Unexpected identifier '{name}'"),
                        position: span.start,
                    });
                }
            }
            _ => {
                return Err(FormulaError {
                    message: format!("Unexpected token: {:?}", span.token),
                    position: span.start,
                });
            }
        };

        self.dec_depth();
        Ok(res)
    }
}

pub fn validate_formula(expression: &str) -> Result<FormulaInfo, FormulaError> {
    let expr = expression.trim();
    if expr.is_empty() {
        return Err(FormulaError {
            message: "Formula expression is empty".to_string(),
            position: 0,
        });
    }
    if expr.len() > MAX_FORMULA_CHARS {
        return Err(FormulaError {
            message: format!("Formula exceeds maximum length of {MAX_FORMULA_CHARS} characters"),
            position: MAX_FORMULA_CHARS,
        });
    }

    let tokens = tokenize_formula(expr)?;
    let mut parser = AstParser::new(tokens);
    let ast = parser.parse_all()?;
    let node_count = ast.count_nodes();
    if node_count > MAX_FORMULA_NODES {
        return Err(FormulaError {
            message: format!("Formula exceeds maximum node limit of {MAX_FORMULA_NODES}"),
            position: 0,
        });
    }

    let mut refs = HashSet::new();
    ast.collect_field_refs(&mut refs);
    let mut field_refs: Vec<String> = refs.into_iter().collect();
    field_refs.sort();

    Ok(FormulaInfo {
        node_count,
        field_refs,
    })
}

pub fn formula_field_refs(expression: &str) -> Option<Vec<String>> {
    validate_formula(expression).ok().map(|info| info.field_refs)
}

pub fn evaluate_formula(expression: &str, record: &Value) -> Value {
    let expr = expression.trim();
    if expr.is_empty() || expr.len() > MAX_FORMULA_CHARS {
        return Value::Null;
    }

    let tokens = match tokenize_formula(expr) {
        Ok(t) => t,
        Err(_) => return Value::Null,
    };

    let mut parser = AstParser::new(tokens);
    let ast = match parser.parse_all() {
        Ok(a) => a,
        Err(_) => return Value::Null,
    };

    if ast.count_nodes() > MAX_FORMULA_NODES {
        return Value::Null;
    }

    let mut evaluator = AstEvaluator::new(record);
    let val = evaluator.eval(&ast);
    truncate_result(val)
}

pub fn parse_operand(op: &str, record: &Value) -> Value {
    evaluate_formula(op, record)
}

struct AstEvaluator<'a> {
    record: &'a Value,
}

impl<'a> AstEvaluator<'a> {
    fn new(record: &'a Value) -> Self {
        Self { record }
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

    fn is_blank(val: &Value) -> bool {
        match val {
            Value::Null => true,
            Value::String(s) => s.is_empty(),
            _ => false,
        }
    }

    fn eval(&mut self, node: &AstNode) -> Value {
        match node {
            AstNode::Number(n) => Value::from(*n),
            AstNode::String(s) => Value::from(s.clone()),
            AstNode::Field(name) => self.record.get(name).cloned().unwrap_or(Value::Null),
            AstNode::UnaryMinus(inner) => {
                let val = self.eval(inner);
                if let Some(n) = Self::to_number(&val) {
                    Value::from(-n)
                } else {
                    Value::Null
                }
            }
            AstNode::BinaryOp { op, left, right } => self.eval_binary_op(op, left, right),
            AstNode::Call { name, args } => self.eval_call(name, args),
        }
    }

    fn eval_binary_op(&mut self, op: &FormulaToken, left_node: &AstNode, right_node: &AstNode) -> Value {
        let left = self.eval(left_node);
        let right = self.eval(right_node);

        match op {
            FormulaToken::Plus => {
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
                    Value::from(s)
                } else if let (Some(nl), Some(nr)) = (Self::to_number(&left), Self::to_number(&right)) {
                    Value::from(nl + nr)
                } else {
                    Value::Null
                }
            }
            FormulaToken::Minus => {
                if let (Some(nl), Some(nr)) = (Self::to_number(&left), Self::to_number(&right)) {
                    Value::from(nl - nr)
                } else {
                    Value::Null
                }
            }
            FormulaToken::Star => {
                if let (Some(nl), Some(nr)) = (Self::to_number(&left), Self::to_number(&right)) {
                    Value::from(nl * nr)
                } else {
                    Value::Null
                }
            }
            FormulaToken::Slash => {
                if let (Some(nl), Some(nr)) = (Self::to_number(&left), Self::to_number(&right)) {
                    if nr == 0.0 {
                        Value::Null
                    } else {
                        Value::from(nl / nr)
                    }
                } else {
                    Value::Null
                }
            }
            FormulaToken::Eq
            | FormulaToken::NotEq
            | FormulaToken::Lt
            | FormulaToken::LtEq
            | FormulaToken::Gt
            | FormulaToken::GtEq => self.compare(&left, &right, op),
            FormulaToken::Ident(id) if id.eq_ignore_ascii_case("AND") => {
                Value::Bool(Self::is_truthy(&left) && Self::is_truthy(&right))
            }
            FormulaToken::Ident(id) if id.eq_ignore_ascii_case("OR") => {
                Value::Bool(Self::is_truthy(&left) || Self::is_truthy(&right))
            }
            _ => Value::Null,
        }
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

    fn eval_call(&mut self, name: &str, args: &[AstNode]) -> Value {
        let evaluated_args: Vec<Value> = args.iter().map(|a| self.eval(a)).collect();
        match name {
            // Logic
            "IF" => {
                let cond = evaluated_args.first().unwrap_or(&Value::Null);
                if Self::is_truthy(cond) {
                    evaluated_args.get(1).cloned().unwrap_or(Value::Null)
                } else {
                    evaluated_args.get(2).cloned().unwrap_or(Value::Null)
                }
            }
            "SWITCH" => {
                if evaluated_args.is_empty() {
                    return Value::Null;
                }
                let target = &evaluated_args[0];
                let mut i = 1;
                while i + 1 < evaluated_args.len() {
                    let case_val = &evaluated_args[i];
                    let out_val = &evaluated_args[i + 1];
                    if self.values_equal(target, case_val) {
                        return out_val.clone();
                    }
                    i += 2;
                }
                if i < evaluated_args.len() {
                    evaluated_args[i].clone()
                } else {
                    Value::Null
                }
            }
            "AND" => {
                if evaluated_args.is_empty() {
                    Value::Null
                } else {
                    Value::Bool(evaluated_args.iter().all(Self::is_truthy))
                }
            }
            "OR" => {
                if evaluated_args.is_empty() {
                    Value::Null
                } else {
                    Value::Bool(evaluated_args.iter().any(Self::is_truthy))
                }
            }
            "NOT" => {
                if evaluated_args.is_empty() {
                    Value::Null
                } else {
                    Value::Bool(!Self::is_truthy(&evaluated_args[0]))
                }
            }
            "ISBLANK" => {
                let val = evaluated_args.first().unwrap_or(&Value::Null);
                Value::Bool(Self::is_blank(val))
            }
            "BLANK" => Value::Null,

            // Math
            "ROUND" => {
                if let Some(n) = evaluated_args.first().and_then(Self::to_number) {
                    let digits = evaluated_args.get(1).and_then(Self::to_number).unwrap_or(0.0) as i32;
                    let factor = 10f64.powi(digits);
                    let rounded = (n * factor).round() / factor;
                    Value::from(rounded)
                } else {
                    Value::Null
                }
            }
            "CEILING" => {
                if let Some(n) = evaluated_args.first().and_then(Self::to_number) {
                    Value::from(n.ceil())
                } else {
                    Value::Null
                }
            }
            "FLOOR" => {
                if let Some(n) = evaluated_args.first().and_then(Self::to_number) {
                    Value::from(n.floor())
                } else {
                    Value::Null
                }
            }
            "ABS" => {
                if let Some(n) = evaluated_args.first().and_then(Self::to_number) {
                    Value::from(n.abs())
                } else {
                    Value::Null
                }
            }
            "MOD" => {
                let n1 = evaluated_args.first().and_then(Self::to_number);
                let n2 = evaluated_args.get(1).and_then(Self::to_number);
                if let (Some(a), Some(b)) = (n1, n2) {
                    if b == 0.0 {
                        Value::Null
                    } else {
                        Value::from(a % b)
                    }
                } else {
                    Value::Null
                }
            }
            "MIN" => {
                let nums: Vec<f64> = evaluated_args.iter().filter_map(Self::to_number).collect();
                if nums.is_empty() {
                    Value::Null
                } else {
                    let m = nums.into_iter().fold(f64::INFINITY, f64::min);
                    Value::from(m)
                }
            }
            "MAX" => {
                let nums: Vec<f64> = evaluated_args.iter().filter_map(Self::to_number).collect();
                if nums.is_empty() {
                    Value::Null
                } else {
                    let m = nums.into_iter().fold(f64::NEG_INFINITY, f64::max);
                    Value::from(m)
                }
            }

            // Text
            "CONCAT" => {
                let mut s = String::new();
                for a in &evaluated_args {
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
                let val = evaluated_args.first().unwrap_or(&Value::Null);
                if val.is_null() {
                    Value::from(0)
                } else if let Some(s) = val.as_str() {
                    Value::from(s.chars().count())
                } else {
                    Value::from(val.to_string().chars().count())
                }
            }
            "LEFT" => {
                let text = evaluated_args.first().and_then(|v| v.as_str()).unwrap_or("");
                let count = evaluated_args.get(1).and_then(Self::to_number).unwrap_or(0.0) as usize;
                let chars: String = text.chars().take(count).collect();
                Value::from(chars)
            }
            "RIGHT" => {
                let text = evaluated_args.first().and_then(|v| v.as_str()).unwrap_or("");
                let count = evaluated_args.get(1).and_then(Self::to_number).unwrap_or(0.0) as usize;
                let total = text.chars().count();
                let skip = total.saturating_sub(count);
                let chars: String = text.chars().skip(skip).collect();
                Value::from(chars)
            }
            "MID" => {
                let text = evaluated_args.first().and_then(|v| v.as_str()).unwrap_or("");
                let start = evaluated_args.get(1).and_then(Self::to_number).unwrap_or(1.0) as usize;
                let count = evaluated_args.get(2).and_then(Self::to_number).unwrap_or(0.0) as usize;
                let skip = start.saturating_sub(1);
                let chars: String = text.chars().skip(skip).take(count).collect();
                Value::from(chars)
            }
            "UPPER" => {
                let text = evaluated_args.first().and_then(|v| v.as_str()).unwrap_or("");
                Value::from(text.to_uppercase())
            }
            "LOWER" => {
                let text = evaluated_args.first().and_then(|v| v.as_str()).unwrap_or("");
                Value::from(text.to_lowercase())
            }
            "TRIM" => {
                let text = evaluated_args.first().and_then(|v| v.as_str()).unwrap_or("");
                Value::from(text.trim().to_string())
            }
            "SUBSTITUTE" => {
                let text = evaluated_args.first().and_then(|v| v.as_str()).unwrap_or("");
                let old = evaluated_args.get(1).and_then(|v| v.as_str()).unwrap_or("");
                let new = evaluated_args.get(2).and_then(|v| v.as_str()).unwrap_or("");
                Value::from(text.replace(old, new))
            }
            "FIND" => {
                let needle = evaluated_args.first().and_then(|v| v.as_str()).unwrap_or("");
                let haystack = evaluated_args.get(1).and_then(|v| v.as_str()).unwrap_or("");
                let start_char = evaluated_args.get(2).and_then(Self::to_number).unwrap_or(1.0) as usize;
                let skip = start_char.saturating_sub(1);
                if needle.is_empty() || haystack.is_empty() {
                    return Value::Null;
                }
                let haystack_chars: Vec<char> = haystack.chars().collect();
                if skip >= haystack_chars.len() {
                    return Value::Null;
                }
                let sub: String = haystack_chars[skip..].iter().collect();
                if let Some(pos) = sub.find(needle) {
                    let char_pos = sub[..pos].chars().count() + skip + 1;
                    Value::from(char_pos)
                } else {
                    Value::Null
                }
            }

            // Date
            "TODAY" => {
                let secs = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let days = (secs / 86400) as i64;
                let (y, m, d) = civil_from_days(days);
                Value::from(format!("{y:04}-{m:02}-{d:02}"))
            }
            "NOW" => {
                let secs = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let days = (secs / 86400) as i64;
                let (y, m, d) = civil_from_days(days);
                let day_secs = secs % 86400;
                let hour = day_secs / 3600;
                let min = (day_secs % 3600) / 60;
                let sec = day_secs % 60;
                Value::from(format!("{y:04}-{m:02}-{d:02}T{hour:02}:{min:02}:{sec:02}Z"))
            }
            "YEAR" => {
                let str_val = evaluated_args.first().and_then(|v| v.as_str());
                if let Some((y, _, _)) = str_val.and_then(parse_date) {
                    Value::from(y)
                } else {
                    Value::Null
                }
            }
            "MONTH" => {
                let str_val = evaluated_args.first().and_then(|v| v.as_str());
                if let Some((_, m, _)) = str_val.and_then(parse_date) {
                    Value::from(m)
                } else {
                    Value::Null
                }
            }
            "DAY" => {
                let str_val = evaluated_args.first().and_then(|v| v.as_str());
                if let Some((_, _, d)) = str_val.and_then(parse_date) {
                    Value::from(d)
                } else {
                    Value::Null
                }
            }
            "WEEKDAY" => {
                let str_val = evaluated_args.first().and_then(|v| v.as_str());
                if let Some((y, m, d)) = str_val.and_then(parse_date) {
                    let days = days_from_civil(y, m, d);
                    let wd = ((days + 3) % 7 + 7) % 7 + 1; // 1 = Monday .. 7 = Sunday
                    Value::from(wd)
                } else {
                    Value::Null
                }
            }
            "DATEADD" => {
                let str_val = evaluated_args.first().and_then(|v| v.as_str());
                let num = evaluated_args.get(1).and_then(Self::to_number).unwrap_or(0.0) as i64;
                let unit = evaluated_args.get(2).and_then(|v| v.as_str()).unwrap_or("days");
                if let Some((y, m, d)) = str_val.and_then(parse_date) {
                    let (ny, nm, nd) = match unit.to_ascii_lowercase().as_str() {
                        "days" | "day" | "d" => {
                            let days = days_from_civil(y, m, d) + num;
                            civil_from_days(days)
                        }
                        "years" | "year" | "y" => {
                            let ny = y + num as i32;
                            let max_d = days_in_month(ny, m);
                            (ny, m, d.min(max_d))
                        }
                        "months" | "month" | "m" => {
                            let total_m = (y as i64) * 12 + (m as i64 - 1) + num;
                            let ny = (total_m.div_euclid(12)) as i32;
                            let nm = (total_m.rem_euclid(12) + 1) as u32;
                            let max_d = days_in_month(ny, nm);
                            (ny, nm, d.min(max_d))
                        }
                        _ => (y, m, d),
                    };
                    Value::from(format!("{ny:04}-{nm:02}-{nd:02}"))
                } else {
                    Value::Null
                }
            }
            "DATEDIFF" => {
                let d1_str = evaluated_args.first().and_then(|v| v.as_str());
                let d2_str = evaluated_args.get(1).and_then(|v| v.as_str());
                let unit = evaluated_args.get(2).and_then(|v| v.as_str()).unwrap_or("days");
                if let (Some((y1, m1, day1)), Some((y2, m2, day2))) = (d1_str.and_then(parse_date), d2_str.and_then(parse_date)) {
                    let diff_days = days_from_civil(y1, m1, day1) - days_from_civil(y2, m2, day2);
                    match unit.to_ascii_lowercase().as_str() {
                        "days" | "day" | "d" => Value::from(diff_days),
                        "months" | "month" | "m" => {
                            let m_diff = (y1 as i64 - y2 as i64) * 12 + (m1 as i64 - m2 as i64);
                            Value::from(m_diff)
                        }
                        "years" | "year" | "y" => Value::from(y1 - y2),
                        _ => Value::from(diff_days),
                    }
                } else {
                    Value::Null
                }
            }
            "FORMAT" => {
                let val = evaluated_args.first().unwrap_or(&Value::Null);
                let fmt = evaluated_args.get(1).and_then(|v| v.as_str()).unwrap_or("");
                if let Some(s) = val.as_str() {
                    if let Some((y, m, d)) = parse_date(s) {
                        let res = fmt
                            .replace("YYYY", &format!("{y:04}"))
                            .replace("MM", &format!("{m:02}"))
                            .replace("DD", &format!("{d:02}"));
                        Value::from(res)
                    } else {
                        Value::from(s.to_string())
                    }
                } else if !val.is_null() {
                    Value::from(val.to_string())
                } else {
                    Value::Null
                }
            }

            // List
            "ARRAYJOIN" => {
                let sep = evaluated_args.get(1).and_then(|v| v.as_str()).unwrap_or(", ");
                let target = evaluated_args.first().unwrap_or(&Value::Null);
                match target {
                    Value::Array(arr) => {
                        let items: Vec<String> = arr.iter().filter_map(|v| match v {
                            Value::String(s) => Some(s.clone()),
                            Value::Null => None,
                            _ => Some(v.to_string()),
                        }).collect();
                        Value::from(items.join(sep))
                    }
                    Value::String(s) => Value::from(s.clone()),
                    Value::Null => Value::from(""),
                    _ => Value::from(target.to_string()),
                }
            }
            "ARRAYUNIQUE" => {
                let target = evaluated_args.first().unwrap_or(&Value::Null);
                match target {
                    Value::Array(arr) => {
                        let mut unique = Vec::new();
                        for item in arr {
                            if !unique.contains(item) {
                                unique.push(item.clone());
                            }
                        }
                        Value::Array(unique)
                    }
                    Value::Null => Value::Array(vec![]),
                    _ => Value::Array(vec![target.clone()]),
                }
            }
            "COUNT" => {
                let target = evaluated_args.first().unwrap_or(&Value::Null);
                match target {
                    Value::Array(arr) => {
                        let cnt = arr.iter().filter(|v| !v.is_null()).count();
                        Value::from(cnt)
                    }
                    Value::Null => Value::from(0),
                    _ => Value::from(1),
                }
            }
            "SUM" => {
                let target = evaluated_args.first().unwrap_or(&Value::Null);
                match target {
                    Value::Array(arr) => {
                        let s: f64 = arr.iter().filter_map(Self::to_number).sum();
                        Value::from(s)
                    }
                    Value::Null => Value::from(0.0),
                    _ => {
                        if let Some(n) = Self::to_number(target) {
                            Value::from(n)
                        } else {
                            Value::from(0.0)
                        }
                    }
                }
            }
            "AVERAGE" => {
                let target = evaluated_args.first().unwrap_or(&Value::Null);
                match target {
                    Value::Array(arr) => {
                        let nums: Vec<f64> = arr.iter().filter_map(Self::to_number).collect();
                        if nums.is_empty() {
                            Value::Null
                        } else {
                            let avg = nums.iter().sum::<f64>() / nums.len() as f64;
                            Value::from(avg)
                        }
                    }
                    Value::Null => Value::Null,
                    _ => Self::to_number(target).map(Value::from).unwrap_or(Value::Null),
                }
            }

            _ => Value::Null,
        }
    }

    fn values_equal(&self, a: &Value, b: &Value) -> bool {
        if a.is_null() && b.is_null() {
            return true;
        }
        if let (Some(na), Some(nb)) = (Self::to_number(a), Self::to_number(b)) {
            return na == nb;
        }
        if let (Some(sa), Some(sb)) = (a.as_str(), b.as_str()) {
            return sa == sb;
        }
        a == b
    }
}

fn truncate_result(val: Value) -> Value {
    match val {
        Value::String(s) if s.len() > MAX_RESULT_TEXT_CHARS => {
            let truncated: String = s.chars().take(MAX_RESULT_TEXT_CHARS).collect();
            Value::String(truncated)
        }
        _ => val,
    }
}

fn parse_date(s: &str) -> Option<(i32, u32, u32)> {
    if s.len() < 10 {
        return None;
    }
    let date_part = &s[..10];
    let parts: Vec<&str> = date_part.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    let y = parts[0].parse::<i32>().ok()?;
    let m = parts[1].parse::<u32>().ok()?;
    let d = parts[2].parse::<u32>().ok()?;
    if (1..=12).contains(&m) && (1..=31).contains(&d) {
        Some((y, m, d))
    } else {
        None
    }
}

fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            let is_leap = (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0);
            if is_leap {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = y - if m <= 2 { 1 } else { 0 };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = (y - era * 400) as u32;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era as i64) * 146097 + (doe as i64) - 719468
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}
