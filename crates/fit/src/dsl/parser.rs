//! Recursive-descent expression parser with depth and node limits.
//!
//! References:
//! - Aho, A. V., Lam, M. S., Sethi, R. & Ullman, J. D. (2006). *Compilers: Principles, Techniques, and
//!   Tools*, 2nd ed. Addison-Wesley.

use std::collections::HashMap;

use crate::dsl::ast::{BinaryOp, Expr, Token, TokenKind, UnaryOp};
use crate::dsl::lexer::tokenize;
use crate::dsl::limits::{MAX_AST_NODES, MAX_PARSE_DEPTH};
use crate::error::FitError;

pub(in crate::dsl) struct Parser<'a> {
    pub(in crate::dsl) tokens: &'a [Token],
    pub(in crate::dsl) pos: usize,
    pub(in crate::dsl) line: usize,
    pub(in crate::dsl) depth: usize,
    pub(in crate::dsl) nodes: usize,
}

impl<'a> Parser<'a> {
    pub(in crate::dsl) fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }
    pub(in crate::dsl) fn bump(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos)?.clone();
        self.pos += 1;
        Some(t)
    }
    pub(in crate::dsl) fn track_node(&mut self, line: usize, col: usize) -> Result<(), FitError> {
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > MAX_AST_NODES {
            return Err(FitError::DslCompileError { message: format!("Expression AST exceeds maximum nodes ({MAX_AST_NODES})"), line, column: col });
        }
        Ok(())
    }
    pub(in crate::dsl) fn enter_depth(&mut self, line: usize, col: usize) -> Result<(), FitError> {
        if self.depth >= MAX_PARSE_DEPTH {
            return Err(FitError::DslCompileError {
                message: format!("Expression nesting exceeds maximum depth ({MAX_PARSE_DEPTH})"),
                line,
                column: col,
            });
        }
        self.depth += 1;
        Ok(())
    }
    pub(in crate::dsl) fn leave_depth(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }
    pub(in crate::dsl) fn parse_or(&mut self) -> Result<Expr, FitError> {
        let mut lhs = self.parse_and()?;
        while let Some((tok_line, tok_col)) = self.peek().filter(|t| t.kind == TokenKind::OrOr).map(|t| (t.line, t.col)) {
            self.bump();
            self.track_node(tok_line, tok_col)?;
            let rhs = self.parse_and()?;
            lhs = Expr::Binary { op: BinaryOp::Or, lhs: Box::new(lhs), rhs: Box::new(rhs) };
        }
        Ok(lhs)
    }
    pub(in crate::dsl) fn parse_and(&mut self) -> Result<Expr, FitError> {
        let mut lhs = self.parse_cmp()?;
        while let Some((tok_line, tok_col)) = self.peek().filter(|t| t.kind == TokenKind::AndAnd).map(|t| (t.line, t.col)) {
            self.bump();
            self.track_node(tok_line, tok_col)?;
            let rhs = self.parse_cmp()?;
            lhs = Expr::Binary { op: BinaryOp::And, lhs: Box::new(lhs), rhs: Box::new(rhs) };
        }
        Ok(lhs)
    }
    pub(in crate::dsl) fn parse_cmp(&mut self) -> Result<Expr, FitError> {
        let mut lhs = self.parse_add()?;
        while let Some(tok) = self.peek() {
            let (tok_line, tok_col) = (tok.line, tok.col);
            let op = match tok.kind {
                TokenKind::EqEq | TokenKind::Eq => BinaryOp::Eq,
                TokenKind::NotEq => BinaryOp::Ne,
                TokenKind::Lt => BinaryOp::Lt,
                TokenKind::Le => BinaryOp::Le,
                TokenKind::Gt => BinaryOp::Gt,
                TokenKind::Ge => BinaryOp::Ge,
                _ => break,
            };
            self.bump();
            self.track_node(tok_line, tok_col)?;
            let rhs = self.parse_add()?;
            lhs = Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) };
        }
        Ok(lhs)
    }
    pub(in crate::dsl) fn parse_add(&mut self) -> Result<Expr, FitError> {
        let mut lhs = self.parse_mul()?;
        while let Some(tok) = self.peek() {
            let (tok_line, tok_col) = (tok.line, tok.col);
            let op = match tok.kind {
                TokenKind::Plus => BinaryOp::Add,
                TokenKind::Minus => BinaryOp::Sub,
                _ => break,
            };
            self.bump();
            self.track_node(tok_line, tok_col)?;
            let rhs = self.parse_mul()?;
            lhs = Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) };
        }
        Ok(lhs)
    }
    pub(in crate::dsl) fn parse_mul(&mut self) -> Result<Expr, FitError> {
        let mut lhs = self.parse_unary()?;
        while let Some(tok) = self.peek() {
            let (tok_line, tok_col) = (tok.line, tok.col);
            let op = match tok.kind {
                TokenKind::Star => BinaryOp::Mul,
                TokenKind::Slash => BinaryOp::Div,
                TokenKind::Percent => BinaryOp::Rem,
                _ => break,
            };
            self.bump();
            self.track_node(tok_line, tok_col)?;
            let rhs = self.parse_unary()?;
            lhs = Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) };
        }
        Ok(lhs)
    }
    /// Parse prefix unary (`+`, `-`, `!`) and right-associative exponentiation (`^`), with `^`
    /// binding tighter than prefix unary operators (`-X^2` -> `-(X^2)`) while permitting prefix
    /// unary operators in the right-hand exponent (`X^-2` -> `X^(-2)`).
    pub(in crate::dsl) fn parse_unary(&mut self) -> Result<Expr, FitError> {
        let mut segments: Vec<(Vec<UnaryOp>, Expr)> = Vec::new();
        let mut total_chain_depth = 0_usize;
        loop {
            let mut ops: Vec<UnaryOp> = Vec::new();
            while let Some(tok) = self.peek() {
                let (tok_line, tok_col, kind) = (tok.line, tok.col, tok.kind);
                match kind {
                    TokenKind::Plus => {
                        self.bump();
                    }
                    TokenKind::Minus => {
                        self.bump();
                        self.track_node(tok_line, tok_col)?;
                        ops.push(UnaryOp::Neg);
                    }
                    TokenKind::Bang => {
                        self.bump();
                        self.track_node(tok_line, tok_col)?;
                        ops.push(UnaryOp::Not);
                    }
                    _ => break,
                }
                total_chain_depth += 1;
                if self.depth + total_chain_depth > MAX_PARSE_DEPTH {
                    return Err(FitError::DslCompileError {
                        message: format!("Expression nesting exceeds maximum depth ({MAX_PARSE_DEPTH})"),
                        line: tok_line,
                        column: tok_col,
                    });
                }
            }
            let saved_depth = self.depth;
            self.depth = saved_depth + total_chain_depth;
            let primary_res = self.parse_primary();
            self.depth = saved_depth;
            let primary = primary_res?;
            segments.push((ops, primary));

            let Some(tok) = self.peek() else {
                break;
            };
            if tok.kind != TokenKind::Caret {
                break;
            }
            let (tok_line, tok_col) = (tok.line, tok.col);
            self.bump();
            total_chain_depth += 1;
            if self.depth + total_chain_depth > MAX_PARSE_DEPTH {
                return Err(FitError::DslCompileError {
                    message: format!("Expression nesting exceeds maximum depth ({MAX_PARSE_DEPTH})"),
                    line: tok_line,
                    column: tok_col,
                });
            }
            self.track_node(tok_line, tok_col)?;
        }

        let (mut last_ops, mut acc) = segments.pop().ok_or_else(|| FitError::DslCompileError {
            message: "Unexpected end of expression".to_owned(),
            line: self.line,
            column: 1,
        })?;
        while let Some(op) = last_ops.pop() {
            acc = Expr::Unary { op, rhs: Box::new(acc) };
        }
        while let Some((mut ops, base)) = segments.pop() {
            acc = Expr::Binary { op: BinaryOp::Pow, lhs: Box::new(base), rhs: Box::new(acc) };
            while let Some(op) = ops.pop() {
                acc = Expr::Unary { op, rhs: Box::new(acc) };
            }
        }
        Ok(acc)
    }
    pub(in crate::dsl) fn parse_call_args(&mut self, tok: &Token) -> Result<Vec<Expr>, FitError> {
        self.bump();
        let mut args = Vec::new();
        if !self.peek().is_some_and(|t| t.kind == TokenKind::RParen) {
            loop {
                args.push(self.parse_or()?);
                if self.peek().is_some_and(|t| t.kind == TokenKind::Comma) {
                    self.bump();
                } else {
                    break;
                }
            }
        }
        if !self.bump().is_some_and(|t| t.kind == TokenKind::RParen) {
            return Err(FitError::DslCompileError { message: format!("Expected `)` after call to `{}`", tok.text), line: tok.line, column: tok.col });
        }
        Ok(args)
    }
    pub(in crate::dsl) fn build_call_expr(tok: Token, args: Vec<Expr>) -> Result<Expr, FitError> {
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
    pub(in crate::dsl) fn parse_primary(&mut self) -> Result<Expr, FitError> {
        let Some(tok) = self.bump() else {
            return Err(FitError::DslCompileError { message: "Unexpected end of expression".to_owned(), line: self.line, column: 1 });
        };
        match tok.kind {
            TokenKind::Number => {
                self.track_node(tok.line, tok.col)?;
                Ok(Expr::Num(tok.num))
            }
            TokenKind::LParen => {
                self.enter_depth(tok.line, tok.col)?;
                let inner = self.parse_or();
                self.leave_depth();
                let inner = inner?;
                match self.bump() {
                    Some(t) if t.kind == TokenKind::RParen => Ok(inner),
                    _ => Err(FitError::DslCompileError { message: "Expected `)`".to_owned(), line: tok.line, column: tok.col }),
                }
            }
            TokenKind::Ident => {
                if self.peek().is_some_and(|t| t.kind == TokenKind::LParen) {
                    self.track_node(tok.line, tok.col)?;
                    self.enter_depth(tok.line, tok.col)?;
                    let args = self.parse_call_args(&tok);
                    self.leave_depth();
                    Self::build_call_expr(tok, args?)
                } else {
                    self.track_node(tok.line, tok.col)?;
                    Ok(Expr::Ident(tok.text))
                }
            }
            _ => Err(FitError::DslCompileError { message: format!("Unexpected token `{}`", tok.text), line: tok.line, column: tok.col }),
        }
    }
}

pub(in crate::dsl) fn parse_expr_collecting_locs(
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
    let mut parser = Parser { tokens: &tokens, pos: 0, line, depth: 0, nodes: 0 };
    let expr = parser.parse_or()?;
    if let Some(rem) = parser.peek() {
        return Err(FitError::DslCompileError { message: format!("Unexpected trailing token `{}`", rem.text), line: rem.line, column: rem.col });
    }
    Ok(expr)
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
