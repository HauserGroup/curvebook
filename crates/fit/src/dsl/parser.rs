//! Iterative operator-stack expression parser with depth and node limits.
//!
//! Uses an explicit operator/operand stack so arbitrarily deep inputs never consume call-stack
//! frames (safe on small worker and `wasm32` stacks) while enforcing [`MAX_PARSE_DEPTH`] and
//! [`MAX_AST_NODES`].
//!
//! References:
//! - Aho, A. V., Lam, M. S., Sethi, R. & Ullman, J. D. (2006). *Compilers: Principles, Techniques, and
//!   Tools*, 2nd ed. Addison-Wesley.

use std::collections::HashMap;

use crate::dsl::ast::{BinaryOp, Expr, Token, TokenKind, UnaryOp};
use crate::dsl::lexer::tokenize;
use crate::dsl::limits::{MAX_AST_NODES, MAX_PARSE_DEPTH};
use crate::error::FitError;

enum ParseOp {
    UnaryPlus,
    Unary(UnaryOp),
    Binary(BinaryOp),
    Paren { line: usize, col: usize },
    CallStart { tok: Token, arg_count: usize },
}

impl ParseOp {
    /// Returns `(precedence, left_associative)` for operators, or `None` for open delimiters.
    fn precedence(&self) -> Option<(u8, bool)> {
        match self {
            Self::Binary(BinaryOp::Or) => Some((1, true)),
            Self::Binary(BinaryOp::And) => Some((2, true)),
            Self::Binary(BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge) => Some((3, true)),
            Self::Binary(BinaryOp::Add | BinaryOp::Sub) => Some((4, true)),
            Self::Binary(BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem) => Some((5, true)),
            Self::UnaryPlus | Self::Unary(_) => Some((6, false)),
            Self::Binary(BinaryOp::Pow) => Some((7, false)),
            Self::Paren { .. } | Self::CallStart { .. } => None,
        }
    }

    fn counts_toward_depth(&self) -> bool {
        matches!(self, Self::UnaryPlus | Self::Unary(_) | Self::Binary(BinaryOp::Pow) | Self::Paren { .. } | Self::CallStart { .. })
    }
}

fn infix_binary_op(kind: TokenKind) -> Option<(BinaryOp, u8, bool)> {
    match kind {
        TokenKind::OrOr => Some((BinaryOp::Or, 1, true)),
        TokenKind::AndAnd => Some((BinaryOp::And, 2, true)),
        TokenKind::EqEq | TokenKind::Eq => Some((BinaryOp::Eq, 3, true)),
        TokenKind::NotEq => Some((BinaryOp::Ne, 3, true)),
        TokenKind::Lt => Some((BinaryOp::Lt, 3, true)),
        TokenKind::Le => Some((BinaryOp::Le, 3, true)),
        TokenKind::Gt => Some((BinaryOp::Gt, 3, true)),
        TokenKind::Ge => Some((BinaryOp::Ge, 3, true)),
        TokenKind::Plus => Some((BinaryOp::Add, 4, true)),
        TokenKind::Minus => Some((BinaryOp::Sub, 4, true)),
        TokenKind::Star => Some((BinaryOp::Mul, 5, true)),
        TokenKind::Slash => Some((BinaryOp::Div, 5, true)),
        TokenKind::Percent => Some((BinaryOp::Rem, 5, true)),
        TokenKind::Caret => Some((BinaryOp::Pow, 7, false)),
        _ => None,
    }
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    line: usize,
    depth: usize,
    nodes: usize,
    ops: Vec<ParseOp>,
    vals: Vec<Expr>,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Token], line: usize) -> Self {
        Self { tokens, pos: 0, line, depth: 0, nodes: 0, ops: Vec::new(), vals: Vec::new() }
    }

    fn track_node(&mut self, line: usize, col: usize) -> Result<(), FitError> {
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > MAX_AST_NODES {
            return Err(FitError::DslCompileError { message: format!("Expression AST exceeds maximum nodes ({MAX_AST_NODES})"), line, column: col });
        }
        Ok(())
    }

    fn check_depth(&self, line: usize, col: usize) -> Result<(), FitError> {
        if self.depth >= MAX_PARSE_DEPTH {
            return Err(FitError::DslCompileError {
                message: format!("Expression nesting exceeds maximum depth ({MAX_PARSE_DEPTH})"),
                line,
                column: col,
            });
        }
        Ok(())
    }

    fn push_op(&mut self, op: ParseOp, line: usize, col: usize) -> Result<(), FitError> {
        if op.counts_toward_depth() {
            self.check_depth(line, col)?;
            self.depth += 1;
        }
        self.ops.push(op);
        Ok(())
    }

    fn pop_op(&mut self) -> Result<Option<ParseOp>, FitError> {
        let Some(op) = self.ops.pop() else {
            return Ok(None);
        };
        if op.counts_toward_depth() {
            self.depth = self.depth.saturating_sub(1);
        }
        Ok(Some(op))
    }

    fn apply_op(&mut self, op: ParseOp) -> Result<(), FitError> {
        match op {
            ParseOp::UnaryPlus => Ok(()),
            ParseOp::Unary(uop) => {
                let rhs = self.vals.pop().ok_or_else(|| FitError::DslCompileError {
                    message: "Unexpected end of expression".to_owned(),
                    line: self.line,
                    column: 1,
                })?;
                self.vals.push(Expr::Unary { op: uop, rhs: Box::new(rhs) });
                Ok(())
            }
            ParseOp::Binary(bop) => {
                let (Some(rhs), Some(lhs)) = (self.vals.pop(), self.vals.pop()) else {
                    return Err(FitError::DslCompileError { message: "Unexpected end of expression".to_owned(), line: self.line, column: 1 });
                };
                self.vals.push(Expr::Binary { op: bop, lhs: Box::new(lhs), rhs: Box::new(rhs) });
                Ok(())
            }
            ParseOp::Paren { line, col } => Err(FitError::DslCompileError { message: "Expected `)`".to_owned(), line, column: col }),
            ParseOp::CallStart { tok, .. } => {
                Err(FitError::DslCompileError { message: format!("Expected `)` after call to `{}`", tok.text), line: tok.line, column: tok.col })
            }
        }
    }

    fn pop_ops_above(&mut self, min_prec: u8, left_assoc: bool) -> Result<(), FitError> {
        while let Some((top_prec, _)) = self.ops.last().and_then(ParseOp::precedence) {
            if top_prec > min_prec || (left_assoc && top_prec == min_prec) {
                if let Some(op) = self.pop_op()? {
                    self.apply_op(op)?;
                }
            } else {
                break;
            }
        }
        Ok(())
    }

    fn build_call_expr(tok: Token, args: Vec<Expr>) -> Result<Expr, FitError> {
        let upper = tok.text.to_ascii_uppercase();
        if upper == "IF" {
            if args.len() != 3 {
                return Err(FitError::DslCompileError {
                    message: "`IF(cond, then, else)` requires 3 arguments".to_owned(),
                    line: tok.line,
                    column: tok.col,
                });
            }
            let mut it = args.into_iter();
            if let (Some(c), Some(t_br), Some(e_br)) = (it.next(), it.next(), it.next()) {
                Ok(Expr::If { cond: Box::new(c), then_br: Box::new(t_br), else_br: Box::new(e_br) })
            } else {
                Err(FitError::DslCompileError { message: "Invalid IF arguments".to_owned(), line: tok.line, column: tok.col })
            }
        } else if upper == "AND" && args.len() == 2 {
            let mut it = args.into_iter();
            if let (Some(a), Some(b)) = (it.next(), it.next()) {
                Ok(Expr::Binary { op: BinaryOp::And, lhs: Box::new(a), rhs: Box::new(b) })
            } else {
                Err(FitError::DslCompileError { message: "Invalid AND arguments".to_owned(), line: tok.line, column: tok.col })
            }
        } else if upper == "OR" && args.len() == 2 {
            let mut it = args.into_iter();
            if let (Some(a), Some(b)) = (it.next(), it.next()) {
                Ok(Expr::Binary { op: BinaryOp::Or, lhs: Box::new(a), rhs: Box::new(b) })
            } else {
                Err(FitError::DslCompileError { message: "Invalid OR arguments".to_owned(), line: tok.line, column: tok.col })
            }
        } else if upper == "NOT" && args.len() == 1 {
            if let Some(a) = args.into_iter().next() {
                Ok(Expr::Unary { op: UnaryOp::Not, rhs: Box::new(a) })
            } else {
                Err(FitError::DslCompileError { message: "Invalid NOT argument".to_owned(), line: tok.line, column: tok.col })
            }
        } else {
            Ok(Expr::Call { name: tok.text, args })
        }
    }

    fn handle_prefix(&mut self) -> Result<bool, FitError> {
        let tok = &self.tokens[self.pos];
        match tok.kind {
            TokenKind::Plus => {
                self.push_op(ParseOp::UnaryPlus, tok.line, tok.col)?;
                self.pos += 1;
                Ok(true)
            }
            TokenKind::Minus => {
                self.track_node(tok.line, tok.col)?;
                self.push_op(ParseOp::Unary(UnaryOp::Neg), tok.line, tok.col)?;
                self.pos += 1;
                Ok(true)
            }
            TokenKind::Bang => {
                self.track_node(tok.line, tok.col)?;
                self.push_op(ParseOp::Unary(UnaryOp::Not), tok.line, tok.col)?;
                self.pos += 1;
                Ok(true)
            }
            TokenKind::LParen => {
                self.push_op(ParseOp::Paren { line: tok.line, col: tok.col }, tok.line, tok.col)?;
                self.pos += 1;
                Ok(true)
            }
            TokenKind::Number => {
                self.track_node(tok.line, tok.col)?;
                self.vals.push(Expr::Num(tok.num));
                self.pos += 1;
                Ok(false)
            }
            TokenKind::Ident => self.handle_ident_prefix(),
            _ => Err(FitError::DslCompileError { message: format!("Unexpected token `{}`", tok.text), line: tok.line, column: tok.col }),
        }
    }

    fn handle_ident_prefix(&mut self) -> Result<bool, FitError> {
        let tok = self.tokens[self.pos].clone();
        self.track_node(tok.line, tok.col)?;
        if self.tokens.get(self.pos + 1).is_some_and(|t| t.kind == TokenKind::LParen) {
            if self.tokens.get(self.pos + 2).is_some_and(|t| t.kind == TokenKind::RParen) {
                self.check_depth(tok.line, tok.col)?;
                let expr = Self::build_call_expr(tok, Vec::new())?;
                self.vals.push(expr);
                self.pos += 3;
                Ok(false)
            } else {
                self.push_op(ParseOp::CallStart { tok: tok.clone(), arg_count: 1 }, tok.line, tok.col)?;
                self.pos += 2;
                Ok(true)
            }
        } else {
            self.vals.push(Expr::Ident(tok.text));
            self.pos += 1;
            Ok(false)
        }
    }

    fn handle_infix(&mut self) -> Result<bool, FitError> {
        let tok = &self.tokens[self.pos];
        if let Some((bop, prec, left_assoc)) = infix_binary_op(tok.kind) {
            let (line, col) = (tok.line, tok.col);
            self.track_node(line, col)?;
            self.pop_ops_above(prec, left_assoc)?;
            self.push_op(ParseOp::Binary(bop), line, col)?;
            self.pos += 1;
            return Ok(true);
        }
        match tok.kind {
            TokenKind::Comma => {
                self.pop_ops_above(1, true)?;
                match self.ops.last_mut() {
                    Some(ParseOp::CallStart { arg_count, .. }) => {
                        *arg_count += 1;
                        self.pos += 1;
                        Ok(true)
                    }
                    _ => Err(FitError::DslCompileError {
                        message: format!("Unexpected trailing token `{}`", tok.text),
                        line: tok.line,
                        column: tok.col,
                    }),
                }
            }
            TokenKind::RParen => {
                let (line, col, text) = (tok.line, tok.col, tok.text.clone());
                self.pop_ops_above(1, true)?;
                match self.pop_op()? {
                    Some(ParseOp::Paren { .. }) => {
                        self.pos += 1;
                        Ok(false)
                    }
                    Some(ParseOp::CallStart { tok: call_tok, arg_count }) => {
                        if self.vals.len() < arg_count {
                            return Err(FitError::DslCompileError {
                                message: format!("Expected `)` after call to `{}`", call_tok.text),
                                line: call_tok.line,
                                column: call_tok.col,
                            });
                        }
                        let args = self.vals.split_off(self.vals.len() - arg_count);
                        let expr = Self::build_call_expr(call_tok, args)?;
                        self.vals.push(expr);
                        self.pos += 1;
                        Ok(false)
                    }
                    _ => Err(FitError::DslCompileError { message: format!("Unexpected trailing token `{text}`"), line, column: col }),
                }
            }
            _ => Err(FitError::DslCompileError { message: format!("Unexpected trailing token `{}`", tok.text), line: tok.line, column: tok.col }),
        }
    }

    fn parse(mut self) -> Result<Expr, FitError> {
        if self.tokens.is_empty() {
            return Err(FitError::DslCompileError { message: "Unexpected end of expression".to_owned(), line: self.line, column: 1 });
        }
        let mut expect_prefix = true;
        while self.pos < self.tokens.len() {
            expect_prefix = if expect_prefix { self.handle_prefix()? } else { self.handle_infix()? };
        }
        if expect_prefix {
            return Err(FitError::DslCompileError { message: "Unexpected end of expression".to_owned(), line: self.line, column: 1 });
        }
        while let Some(op) = self.pop_op()? {
            self.apply_op(op)?;
        }
        if self.vals.len() == 1
            && let Some(expr) = self.vals.pop()
        {
            Ok(expr)
        } else {
            Err(FitError::DslCompileError { message: "Unexpected end of expression".to_owned(), line: self.line, column: 1 })
        }
    }
}

pub(crate) fn parse_expr_collecting_locs(
    input: &str,
    line: usize,
    col_offset: usize,
    ident_locs: Option<&mut HashMap<String, (usize, usize)>>,
) -> Result<Expr, FitError> {
    let tokens = tokenize(input, line)?;
    if let Some(locs) = ident_locs {
        for (idx, tok) in tokens.iter().enumerate() {
            if tok.kind == TokenKind::Ident && !tokens.get(idx + 1).is_some_and(|next| next.kind == TokenKind::LParen) {
                locs.entry(tok.text.clone()).or_insert((tok.line, col_offset + tok.col));
            }
        }
    }
    Parser::new(&tokens, line).parse()
}

pub(in crate::dsl) enum LocalResolver<'a> {
    Flat(&'a HashMap<String, usize>),
    Scoped { any: &'a HashMap<String, (usize, usize)>, specific: Option<&'a HashMap<String, (usize, usize)>> },
}

impl LocalResolver<'_> {
    pub(in crate::dsl) fn slot(&self, name: &str) -> Option<usize> {
        match self {
            Self::Flat(map) => map.get(name).copied(),
            Self::Scoped { any, specific } => match (any.get(name), specific.and_then(|s| s.get(name))) {
                (Some(&(s1, o1)), Some(&(s2, o2))) => Some(if o2 >= o1 { s2 } else { s1 }),
                (Some(&(s, _)), None) | (None, Some(&(s, _))) => Some(s),
                (None, None) => None,
            },
        }
    }
}
