//! Lexer and Pratt recursive-descent parser for CurveBook formulas (`docs/implementation/data-tables-and-transforms.md` §4.1).

use crate::error::TransformError;

const MAX_PARSE_DEPTH: usize = 128;
const MAX_TOKENS: usize = 16_384;

/// Variable or constant reference inside a formula AST.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum VarRef {
    /// Bare identifier such as `X`, `Y`, `A`, `pi`, or `Height_m`.
    Ident(String),
    /// Bracketed identifier such as `[Body Mass]` or `[HotNM]`.
    Bracketed(String),
    /// Explicit `col("Name")` column lookup.
    ColumnByName(String),
    /// Explicit zero-based `col(idx)` column lookup.
    ColumnByIndex(usize),
    /// Explicit `Info("Key")` metadata constant lookup.
    InfoConstant(String),
    /// 1-based row number (`$row`).
    RowOneBased,
    /// 0-based row index (`$index`).
    IndexZeroBased,
}

/// Unary prefix operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    /// Arithmetic negation (`-x`).
    Neg,
    /// Unary plus (`+x`).
    Pos,
    /// Logical negation (`!x` or `NOT x`).
    Not,
}

/// Binary infix operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    /// Addition (`a + b`).
    Add,
    /// Subtraction (`a - b`).
    Sub,
    /// Multiplication (`a * b`).
    Mul,
    /// Division (`a / b`; yields `Missing` when `b == 0`).
    Div,
    /// Floating-point remainder (`a % b`).
    Mod,
    /// Exponentiation (`a ^ b`).
    Pow,
    /// Equality comparison (`a == b` or `a = b`, emitting `1.0` or `0.0`).
    Eq,
    /// Inequality comparison (`a != b` or `a <> b`).
    Ne,
    /// Strictly less than (`a < b`).
    Lt,
    /// Less than or equal (`a <= b`).
    Le,
    /// Strictly greater than (`a > b`).
    Gt,
    /// Greater than or equal (`a >= b`).
    Ge,
    /// Logical conjunction (`a && b` or `a AND b`).
    And,
    /// Logical disjunction (`a || b` or `a OR b`).
    Or,
}

/// Sort direction inside a window `ORDER BY` clause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SortDirection {
    /// Ascending sort order (`ASC`).
    Asc,
    /// Descending sort order (`DESC`).
    Desc,
}

/// Missing/null placement inside a window `ORDER BY` clause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NullsOrder {
    /// Place missing values before finite/non-missing values (`NULLS FIRST`).
    First,
    /// Place missing values after finite/non-missing values (`NULLS LAST`).
    Last,
}

/// One ordering key expression inside `OVER (... ORDER BY ...)`.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderByExpr {
    /// Key expression evaluated per row within the partition.
    pub expr: Expr,
    /// Ascending vs descending sort direction.
    pub direction: SortDirection,
    /// Placement of [`crate::FormulaValue::Missing`] keys relative to non-missing keys.
    pub nulls: NullsOrder,
}

/// Frame endpoint inside `OVER (... ROWS ...)`.
#[derive(Debug, Clone, PartialEq)]
pub enum FrameBound {
    /// Start of the partition (`UNBOUNDED PRECEDING`).
    UnboundedPreceding,
    /// `n` rows before the current row (`<expr> PRECEDING`).
    Preceding(Box<Expr>),
    /// The current row (`CURRENT ROW`).
    CurrentRow,
    /// `n` rows after the current row (`<expr> FOLLOWING`).
    Following(Box<Expr>),
    /// End of the partition (`UNBOUNDED FOLLOWING`).
    UnboundedFollowing,
}

/// Inclusive row-frame bounds inside `OVER (... ROWS ...)`.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowFrame {
    /// Lower row-frame bound.
    pub start: FrameBound,
    /// Upper row-frame bound.
    pub end: FrameBound,
}

/// Window specification parsed from `OVER (...)`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WindowSpec {
    /// Expressions partitioning rows into independent groups (`PARTITION BY ...`).
    pub partition_by: Vec<Expr>,
    /// Intra-partition sort keys (`ORDER BY ...`).
    pub order_by: Vec<OrderByExpr>,
    /// Optional explicit `ROWS BETWEEN ...` sliding window frame; `None` uses the default partition/peer frame.
    pub frame: Option<WindowFrame>,
}

/// Formula abstract syntax tree node.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Numeric literal (`f64`).
    Number(f64),
    /// Quoted string literal.
    Text(String),
    /// Column, scalar, row-index, or `InfoSheet` variable reference.
    Var(VarRef),
    /// Unary prefix operation (`-x`, `+x`, `!x`).
    Unary {
        /// Prefix operator.
        op: UnaryOp,
        /// Operand sub-expression.
        expr: Box<Expr>,
    },
    /// Binary infix operation (`left op right`).
    Binary {
        /// Infix operator.
        op: BinaryOp,
        /// Left-hand operand.
        left: Box<Expr>,
        /// Right-hand operand.
        right: Box<Expr>,
    },
    /// Standard scalar or aggregate function call (`name(args...)`).
    Call {
        /// Function identifier (matched case-insensitively during evaluation).
        name: String,
        /// Positional argument expressions.
        args: Vec<Expr>,
    },
    /// SQL-style analytic/window function call (`name(args...) [FILTER (WHERE ...)] OVER (...)`).
    WindowCall {
        /// Window or aggregate function identifier.
        name: String,
        /// Positional argument expressions.
        args: Vec<Expr>,
        /// Optional `FILTER (WHERE <predicate>)` row mask applied prior to aggregation; `None` includes all frame rows.
        filter: Option<Box<Expr>>,
        /// Optional `OVER (...)` partition, sort, and row-frame specification.
        window: Option<WindowSpec>,
    },
}

fn push_frame_bound_expr(bound: FrameBound, stack: &mut Vec<Expr>) {
    match bound {
        FrameBound::Preceding(e) | FrameBound::Following(e) => stack.push(*e),
        FrameBound::UnboundedPreceding | FrameBound::CurrentRow | FrameBound::UnboundedFollowing => {}
    }
}

impl Drop for Expr {
    fn drop(&mut self) {
        fn drain_children(node: &mut Expr, stack: &mut Vec<Expr>) {
            match node {
                Expr::Number(_) | Expr::Text(_) | Expr::Var(_) => {}
                Expr::Unary { expr, .. } => {
                    stack.push(std::mem::replace(expr.as_mut(), Expr::Number(0.0)));
                }
                Expr::Binary { left, right, .. } => {
                    stack.push(std::mem::replace(left.as_mut(), Expr::Number(0.0)));
                    stack.push(std::mem::replace(right.as_mut(), Expr::Number(0.0)));
                }
                Expr::Call { args, .. } => {
                    stack.append(args);
                }
                Expr::WindowCall { args, filter, window, .. } => {
                    stack.append(args);
                    if let Some(f) = filter.take() {
                        stack.push(*f);
                    }
                    if let Some(mut w) = window.take() {
                        stack.append(&mut w.partition_by);
                        for o in std::mem::take(&mut w.order_by) {
                            stack.push(o.expr);
                        }
                        if let Some(frame) = w.frame.take() {
                            push_frame_bound_expr(frame.start, stack);
                            push_frame_bound_expr(frame.end, stack);
                        }
                    }
                }
            }
        }

        let mut stack = Vec::new();
        drain_children(self, &mut stack);
        while let Some(mut node) = stack.pop() {
            drain_children(&mut node, &mut stack);
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum TokenKind {
    Number(f64),
    Text(String),
    BracketIdent(String),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Bang,
    LParen,
    RParen,
    Comma,
}

#[derive(Debug, Clone)]
struct Token {
    pos: usize,
    kind: TokenKind,
}

/// Parse a formula string into an [`Expr`] AST.
///
/// Strips any leading `=`, enforces nesting and token bounds, and rejects trailing tokens.
pub fn parse_formula(input: &str) -> Result<Expr, TransformError> {
    let after_leading_ws = input.trim_start();
    let after_eq = after_leading_ws.strip_prefix('=').unwrap_or(after_leading_ws);
    let body_with_tail = after_eq.trim_start();
    let body = body_with_tail.trim_end();
    if body.is_empty() {
        return Err(TransformError::ParseError { position: 0, message: String::from("empty formula") });
    }
    let offset = input.len() - body_with_tail.len();
    let tokens = tokenize(body, offset)?;
    let mut parser = Parser { tokens, cursor: 0, depth: 0 };
    let expr = parser.parse_expr(0)?;
    if let Some(tok) = parser.peek() {
        return Err(TransformError::ParseError { position: tok.pos, message: format!("unexpected trailing token `{:?}`", tok.kind) });
    }
    Ok(expr)
}

struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    base_offset: usize,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str, base_offset: usize) -> Self {
        Self { src, bytes: src.as_bytes(), pos: 0, base_offset }
    }

    fn err(&self, rel: usize, msg: impl Into<String>) -> TransformError {
        TransformError::ParseError { position: self.base_offset + rel, message: msg.into() }
    }

    fn peek_byte(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn next_char(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn skip_whitespace(&mut self) {
        while let Some(b) = self.peek_byte() {
            if b.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn lex_number(&mut self) -> Result<Token, TransformError> {
        let start = self.pos;
        while let Some(b) = self.peek_byte() {
            if b.is_ascii_digit() || b == b'.' {
                self.pos += 1;
            } else {
                break;
            }
        }
        if matches!(self.peek_byte(), Some(b'e' | b'E')) {
            let exp_pos = self.pos;
            let next = self.bytes.get(exp_pos + 1).copied();
            let after = self.bytes.get(exp_pos + 2).copied();
            let has_digits =
                next.is_some_and(|c| c.is_ascii_digit()) || (matches!(next, Some(b'+' | b'-')) && after.is_some_and(|c| c.is_ascii_digit()));
            if has_digits {
                self.pos += 1;
                if matches!(self.peek_byte(), Some(b'+' | b'-')) {
                    self.pos += 1;
                }
                while let Some(b) = self.peek_byte() {
                    if b.is_ascii_digit() {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
            }
        }
        let slice = &self.src[start..self.pos];
        let val = slice.parse::<f64>().map_err(|_| self.err(start, format!("invalid numeric literal `{slice}`")))?;
        if !val.is_finite() {
            return Err(self.err(start, format!("numeric literal `{slice}` overflows IEEE-754 f64")));
        }
        Ok(Token { pos: self.base_offset + start, kind: TokenKind::Number(val) })
    }

    fn lex_string(&mut self, quote: u8) -> Result<Token, TransformError> {
        let start = self.pos;
        self.pos += 1;
        let mut out = String::new();
        while let Some(ch) = self.next_char() {
            if ch == quote as char {
                self.pos += ch.len_utf8();
                return Ok(Token { pos: self.base_offset + start, kind: TokenKind::Text(out) });
            }
            if ch == '\\' {
                self.pos += 1;
                let Some(esc) = self.next_char() else {
                    return Err(self.err(start, "unterminated escape sequence in string literal"));
                };
                self.pos += esc.len_utf8();
                let mapped = match esc {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    other => other,
                };
                out.push(mapped);
            } else {
                self.pos += ch.len_utf8();
                out.push(ch);
            }
        }
        Err(self.err(start, "unterminated string literal"))
    }

    fn lex_bracket_ident(&mut self) -> Result<Token, TransformError> {
        let start = self.pos;
        self.pos += 1;
        let inner_start = self.pos;
        while let Some(ch) = self.next_char() {
            if ch == ']' {
                let name = self.src[inner_start..self.pos].trim().to_owned();
                self.pos += 1;
                if name.is_empty() {
                    return Err(self.err(start, "empty bracketed column reference `[]`"));
                }
                return Ok(Token { pos: self.base_offset + start, kind: TokenKind::BracketIdent(name) });
            }
            self.pos += ch.len_utf8();
        }
        Err(self.err(start, "unclosed `[` in column reference"))
    }

    fn lex_ident(&mut self) -> Result<Token, TransformError> {
        let start = self.pos;
        if self.peek_byte() == Some(b'$') {
            self.pos += 1;
        }
        while let Some(ch) = self.next_char() {
            if ch.is_alphanumeric() || ch == '_' {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
        let ident = &self.src[start..self.pos];
        if ident.is_empty() {
            let ch = self.next_char().unwrap_or('?');
            return Err(self.err(start, format!("unexpected character `{ch}`")));
        }
        if ident == "$" {
            return Err(self.err(start, "expected variable name after `$`"));
        }
        Ok(Token { pos: self.base_offset + start, kind: TokenKind::Ident(ident.to_owned()) })
    }

    fn lex_operator(&mut self) -> Result<Token, TransformError> {
        let start = self.pos;
        let Some(b0) = self.peek_byte() else {
            return Err(self.err(start, "unexpected end of formula"));
        };
        let b1 = self.bytes.get(self.pos + 1).copied();
        let (kind, advance) = match (b0, b1) {
            (b'=', Some(b'=')) => (TokenKind::Eq, 2),
            (b'=', _) => (TokenKind::Eq, 1),
            (b'!', Some(b'=')) | (b'<', Some(b'>')) => (TokenKind::Ne, 2),
            (b'!', _) => (TokenKind::Bang, 1),
            (b'<', Some(b'=')) => (TokenKind::Le, 2),
            (b'<', _) => (TokenKind::Lt, 1),
            (b'>', Some(b'=')) => (TokenKind::Ge, 2),
            (b'>', _) => (TokenKind::Gt, 1),
            (b'&', Some(b'&')) => (TokenKind::And, 2),
            (b'|', Some(b'|')) => (TokenKind::Or, 2),
            (b'+', _) => (TokenKind::Plus, 1),
            (b'-', _) => (TokenKind::Minus, 1),
            (b'*', _) => (TokenKind::Star, 1),
            (b'/', _) => (TokenKind::Slash, 1),
            (b'%', _) => (TokenKind::Percent, 1),
            (b'^', _) => (TokenKind::Caret, 1),
            (b'(', _) => (TokenKind::LParen, 1),
            (b')', _) => (TokenKind::RParen, 1),
            (b',', _) => (TokenKind::Comma, 1),
            _ => {
                let ch = self.next_char().unwrap_or('?');
                return Err(self.err(start, format!("unexpected character `{ch}`")));
            }
        };
        self.pos += advance;
        Ok(Token { pos: self.base_offset + start, kind })
    }
}

fn tokenize(src: &str, base_offset: usize) -> Result<Vec<Token>, TransformError> {
    let mut lexer = Lexer::new(src, base_offset);
    let mut tokens = Vec::new();
    loop {
        lexer.skip_whitespace();
        let Some(b) = lexer.peek_byte() else {
            break;
        };
        if tokens.len() >= MAX_TOKENS {
            return Err(lexer.err(lexer.pos, "formula exceeds maximum token limit"));
        }
        let next_is_digit = lexer.bytes.get(lexer.pos + 1).is_some_and(|c| c.is_ascii_digit());
        let tok = if b.is_ascii_digit() || (b == b'.' && next_is_digit) {
            lexer.lex_number()?
        } else if b == b'"' || b == b'\'' {
            lexer.lex_string(b)?
        } else if b == b'[' {
            lexer.lex_bracket_ident()?
        } else if b == b'$'
            || b == b'_'
            || b.is_ascii_alphabetic()
            || (!b.is_ascii() && lexer.next_char().is_some_and(|ch| ch.is_alphabetic() || ch == '_'))
        {
            lexer.lex_ident()?
        } else {
            lexer.lex_operator()?
        };
        tokens.push(tok);
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    cursor: usize,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.cursor)
    }

    fn peek_next_kind(&self) -> Option<&TokenKind> {
        self.tokens.get(self.cursor + 1).map(|t| &t.kind)
    }

    fn advance(&mut self) -> Option<&Token> {
        let tok = self.tokens.get(self.cursor)?;
        self.cursor += 1;
        Some(tok)
    }

    fn current_pos(&self) -> usize {
        self.peek().map_or_else(|| self.tokens.last().map_or(0, |t| t.pos + 1), |t| t.pos)
    }

    fn parse_expr(&mut self, min_bp: u8) -> Result<Expr, TransformError> {
        if self.depth >= MAX_PARSE_DEPTH {
            return Err(TransformError::ParseError { position: self.current_pos(), message: String::from("formula nesting exceeds maximum depth") });
        }
        self.depth += 1;
        let res = self.parse_expr_inner(min_bp);
        self.depth -= 1;
        res
    }

    fn parse_expr_inner(&mut self, min_bp: u8) -> Result<Expr, TransformError> {
        let mut lhs = self.parse_prefix()?;
        while let Some((op, l_bp, r_bp)) = self.peek_infix_op() {
            if l_bp < min_bp {
                break;
            }
            self.advance();
            let rhs = self.parse_expr(r_bp)?;
            lhs = Expr::Binary { op, left: Box::new(lhs), right: Box::new(rhs) };
        }
        Ok(lhs)
    }

    fn peek_infix_op(&self) -> Option<(BinaryOp, u8, u8)> {
        let tok = self.peek()?;
        match &tok.kind {
            TokenKind::Or => Some((BinaryOp::Or, 1, 2)),
            TokenKind::And => Some((BinaryOp::And, 3, 4)),
            TokenKind::Ident(s) if s.eq_ignore_ascii_case("or") => Some((BinaryOp::Or, 1, 2)),
            TokenKind::Ident(s) if s.eq_ignore_ascii_case("and") => Some((BinaryOp::And, 3, 4)),
            TokenKind::Eq => Some((BinaryOp::Eq, 5, 6)),
            TokenKind::Ne => Some((BinaryOp::Ne, 5, 6)),
            TokenKind::Lt => Some((BinaryOp::Lt, 7, 8)),
            TokenKind::Le => Some((BinaryOp::Le, 7, 8)),
            TokenKind::Gt => Some((BinaryOp::Gt, 7, 8)),
            TokenKind::Ge => Some((BinaryOp::Ge, 7, 8)),
            TokenKind::Plus => Some((BinaryOp::Add, 9, 10)),
            TokenKind::Minus => Some((BinaryOp::Sub, 9, 10)),
            TokenKind::Star => Some((BinaryOp::Mul, 11, 12)),
            TokenKind::Slash => Some((BinaryOp::Div, 11, 12)),
            TokenKind::Percent => Some((BinaryOp::Mod, 11, 12)),
            TokenKind::Caret => Some((BinaryOp::Pow, 16, 15)),
            _ => None,
        }
    }

    fn parse_prefix(&mut self) -> Result<Expr, TransformError> {
        let Some(tok) = self.peek().cloned() else {
            return Err(TransformError::ParseError { position: self.current_pos(), message: String::from("unexpected end of formula") });
        };
        match tok.kind {
            TokenKind::Plus => {
                self.advance();
                let rhs = self.parse_expr(13)?;
                Ok(Expr::Unary { op: UnaryOp::Pos, expr: Box::new(rhs) })
            }
            TokenKind::Minus => {
                self.advance();
                let rhs = self.parse_expr(13)?;
                Ok(Expr::Unary { op: UnaryOp::Neg, expr: Box::new(rhs) })
            }
            TokenKind::Bang => {
                self.advance();
                let rhs = self.parse_expr(13)?;
                Ok(Expr::Unary { op: UnaryOp::Not, expr: Box::new(rhs) })
            }
            TokenKind::Ident(ref name) if name.eq_ignore_ascii_case("not") && !matches!(self.peek_next_kind(), Some(TokenKind::LParen)) => {
                self.advance();
                // Word-form `not` binds looser than comparisons (`==`, `!=`, `<`, `<=`, `>`, `>=` at bp 5..8)
                // and tighter than logical `and` (bp 3..4) and `or` (bp 1..2).
                let rhs = self.parse_expr(5)?;
                Ok(Expr::Unary { op: UnaryOp::Not, expr: Box::new(rhs) })
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, TransformError> {
        let Some(tok) = self.advance().cloned() else {
            return Err(TransformError::ParseError { position: self.current_pos(), message: String::from("expected expression") });
        };
        match tok.kind {
            TokenKind::Number(v) => Ok(Expr::Number(v)),
            TokenKind::Text(s) => Ok(Expr::Text(s)),
            TokenKind::BracketIdent(name) => Ok(Expr::Var(VarRef::Bracketed(name))),
            TokenKind::Ident(name) => self.parse_ident_or_call(tok.pos, name),
            TokenKind::LParen => {
                let inner = self.parse_expr(0)?;
                match self.advance() {
                    Some(Token { kind: TokenKind::RParen, .. }) => Ok(inner),
                    Some(other) => Err(TransformError::ParseError { position: other.pos, message: String::from("expected closing `)`") }),
                    None => Err(TransformError::ParseError { position: tok.pos, message: String::from("unclosed `(` in expression") }),
                }
            }
            other => Err(TransformError::ParseError { position: tok.pos, message: format!("unexpected token `{other:?}`") }),
        }
    }

    fn parse_ident_or_call(&mut self, pos: usize, name: String) -> Result<Expr, TransformError> {
        if !matches!(self.peek().map(|t| &t.kind), Some(TokenKind::LParen)) {
            if name.eq_ignore_ascii_case("$row") {
                return Ok(Expr::Var(VarRef::RowOneBased));
            }
            if name.eq_ignore_ascii_case("$index") {
                return Ok(Expr::Var(VarRef::IndexZeroBased));
            }
            return Ok(Expr::Var(VarRef::Ident(name)));
        }
        self.advance();
        let mut args = Vec::new();
        if !matches!(self.peek().map(|t| &t.kind), Some(TokenKind::RParen)) {
            loop {
                args.push(self.parse_expr(0)?);
                match self.peek().map(|t| &t.kind) {
                    Some(TokenKind::Comma) => {
                        self.advance();
                    }
                    Some(TokenKind::RParen) => break,
                    Some(_) => {
                        return Err(TransformError::ParseError {
                            position: self.current_pos(),
                            message: String::from("expected `,` or `)` in function arguments"),
                        });
                    }
                    None => {
                        return Err(TransformError::ParseError { position: pos, message: String::from("unclosed `(` in function call") });
                    }
                }
            }
        }
        let _ = self.advance();
        let expr = normalize_special_call(pos, name, args)?;
        self.parse_call_postfix_clauses(expr)
    }

    fn peek_ident_eq(&self, kw: &str) -> bool {
        matches!(self.peek().map(|t| &t.kind), Some(TokenKind::Ident(s)) if s.eq_ignore_ascii_case(kw))
    }

    fn eat_ident_eq(&mut self, kw: &str) -> bool {
        if self.peek_ident_eq(kw) {
            let _ = self.advance();
            true
        } else {
            false
        }
    }

    fn expect_ident_eq(&mut self, kw: &str, msg: &str) -> Result<(), TransformError> {
        if self.eat_ident_eq(kw) { Ok(()) } else { Err(TransformError::ParseError { position: self.current_pos(), message: String::from(msg) }) }
    }

    fn expect_lparen(&mut self, msg: &str) -> Result<usize, TransformError> {
        match self.peek().cloned() {
            Some(Token { kind: TokenKind::LParen, pos }) => {
                let _ = self.advance();
                Ok(pos)
            }
            _ => Err(TransformError::ParseError { position: self.current_pos(), message: String::from(msg) }),
        }
    }

    fn expect_rparen(&mut self, open_pos: usize, ctx_label: &str) -> Result<(), TransformError> {
        match self.advance() {
            Some(Token { kind: TokenKind::RParen, .. }) => Ok(()),
            Some(other) => Err(TransformError::ParseError { position: other.pos, message: format!("expected closing `)` in {ctx_label}") }),
            None => Err(TransformError::ParseError { position: open_pos, message: format!("unclosed `(` in {ctx_label}") }),
        }
    }

    fn parse_call_postfix_clauses(&mut self, mut expr: Expr) -> Result<Expr, TransformError> {
        let has_modifier = self.peek_ident_eq("over") || self.peek_ident_eq("where") || self.peek_ident_eq("filter");
        if !has_modifier {
            return Ok(expr);
        }
        let (name, args) = match &mut expr {
            Expr::Call { name, args } => (std::mem::take(name), std::mem::take(args)),
            _ => {
                return Err(TransformError::ParseError {
                    position: self.current_pos(),
                    message: String::from("window or filter clause cannot be attached to `col(...)` or `Info(...)`"),
                });
            }
        };

        let mut filter: Option<Box<Expr>> = None;
        let mut window: Option<WindowSpec> = None;

        loop {
            if self.peek_ident_eq("filter") {
                if filter.is_some() {
                    return Err(TransformError::ParseError {
                        position: self.current_pos(),
                        message: String::from("duplicate `FILTER` / `WHERE` clause on function call"),
                    });
                }
                let _ = self.advance();
                let open_pos = self.expect_lparen("expected `(` after `FILTER`")?;
                self.expect_ident_eq("where", "expected `WHERE` inside `FILTER (...)`")?;
                let cond = self.parse_expr(0)?;
                self.expect_rparen(open_pos, "`FILTER (...)` clause")?;
                filter = Some(Box::new(cond));
            } else if self.peek_ident_eq("where") {
                if filter.is_some() {
                    return Err(TransformError::ParseError {
                        position: self.current_pos(),
                        message: String::from("duplicate `WHERE` / `FILTER` clause on function call"),
                    });
                }
                let _ = self.advance();
                let cond = self.parse_expr(0)?;
                filter = Some(Box::new(cond));
            } else if self.peek_ident_eq("over") {
                if window.is_some() {
                    return Err(TransformError::ParseError {
                        position: self.current_pos(),
                        message: String::from("duplicate `OVER` clause on function call"),
                    });
                }
                let _ = self.advance();
                let open_pos = self.expect_lparen("expected `(` after `OVER`")?;
                let (spec, inline_filter) = self.parse_over_body(open_pos)?;
                if let Some(cond) = inline_filter {
                    if filter.is_some() {
                        return Err(TransformError::ParseError {
                            position: open_pos,
                            message: String::from("duplicate `WHERE` / `FILTER` clause on windowed call"),
                        });
                    }
                    filter = Some(cond);
                }
                window = Some(spec);
            } else {
                break;
            }
        }

        if filter.is_none() && window.is_none() { Ok(Expr::Call { name, args }) } else { Ok(Expr::WindowCall { name, args, filter, window }) }
    }

    fn parse_over_body(&mut self, open_pos: usize) -> Result<(WindowSpec, Option<Box<Expr>>), TransformError> {
        if self.depth >= MAX_PARSE_DEPTH {
            return Err(TransformError::ParseError { position: open_pos, message: String::from("formula nesting exceeds maximum depth") });
        }
        self.depth += 1;
        let res = self.parse_over_body_inner(open_pos);
        self.depth -= 1;
        res
    }

    fn parse_over_body_inner(&mut self, open_pos: usize) -> Result<(WindowSpec, Option<Box<Expr>>), TransformError> {
        let mut spec = WindowSpec::default();
        let mut inline_filter: Option<Box<Expr>> = None;
        let mut seen_partition = false;
        let mut seen_order = false;
        let mut seen_frame = false;

        loop {
            let Some(tok) = self.peek().cloned() else {
                return Err(TransformError::ParseError { position: open_pos, message: String::from("unclosed `(` in `OVER` clause") });
            };
            match tok.kind {
                TokenKind::RParen => {
                    let _ = self.advance();
                    return Ok((spec, inline_filter));
                }
                TokenKind::Ident(kw) => {
                    if kw.eq_ignore_ascii_case("partition") {
                        if seen_partition {
                            return Err(TransformError::ParseError {
                                position: tok.pos,
                                message: String::from("duplicate `PARTITION BY` in `OVER` clause"),
                            });
                        }
                        let _ = self.advance();
                        self.expect_ident_eq("by", "expected `BY` after `PARTITION` in `OVER` clause")?;
                        loop {
                            spec.partition_by.push(self.parse_expr(0)?);
                            if matches!(self.peek().map(|t| &t.kind), Some(TokenKind::Comma)) {
                                let _ = self.advance();
                            } else {
                                break;
                            }
                        }
                        seen_partition = true;
                    } else if kw.eq_ignore_ascii_case("where") {
                        if inline_filter.is_some() {
                            return Err(TransformError::ParseError {
                                position: tok.pos,
                                message: String::from("duplicate `WHERE` in `OVER` clause"),
                            });
                        }
                        let _ = self.advance();
                        inline_filter = Some(Box::new(self.parse_expr(0)?));
                    } else if kw.eq_ignore_ascii_case("filter") {
                        if inline_filter.is_some() {
                            return Err(TransformError::ParseError {
                                position: tok.pos,
                                message: String::from("duplicate `FILTER` in `OVER` clause"),
                            });
                        }
                        let _ = self.advance();
                        let filter_open = self.expect_lparen("expected `(` after `FILTER`")?;
                        self.expect_ident_eq("where", "expected `WHERE` inside `FILTER (...)`")?;
                        inline_filter = Some(Box::new(self.parse_expr(0)?));
                        self.expect_rparen(filter_open, "`FILTER (...)` clause")?;
                    } else if kw.eq_ignore_ascii_case("order") {
                        if seen_order {
                            return Err(TransformError::ParseError {
                                position: tok.pos,
                                message: String::from("duplicate `ORDER BY` in `OVER` clause"),
                            });
                        }
                        let _ = self.advance();
                        self.expect_ident_eq("by", "expected `BY` after `ORDER` in `OVER` clause")?;
                        loop {
                            let expr = self.parse_expr(0)?;
                            let direction = if self.eat_ident_eq("desc") {
                                SortDirection::Desc
                            } else {
                                let _ = self.eat_ident_eq("asc");
                                SortDirection::Asc
                            };
                            let nulls = if self.eat_ident_eq("nulls") {
                                if self.eat_ident_eq("first") {
                                    NullsOrder::First
                                } else {
                                    self.expect_ident_eq("last", "expected `FIRST` or `LAST` after `NULLS`")?;
                                    NullsOrder::Last
                                }
                            } else {
                                NullsOrder::Last
                            };
                            spec.order_by.push(OrderByExpr { expr, direction, nulls });
                            if matches!(self.peek().map(|t| &t.kind), Some(TokenKind::Comma)) {
                                let _ = self.advance();
                            } else {
                                break;
                            }
                        }
                        seen_order = true;
                    } else if kw.eq_ignore_ascii_case("rows") {
                        if seen_frame {
                            return Err(TransformError::ParseError {
                                position: tok.pos,
                                message: String::from("duplicate `ROWS` frame in `OVER` clause"),
                            });
                        }
                        let _ = self.advance();
                        spec.frame = Some(self.parse_window_frame(tok.pos)?);
                        seen_frame = true;
                    } else {
                        return Err(TransformError::ParseError { position: tok.pos, message: format!("unexpected keyword `{kw}` in `OVER` clause") });
                    }
                }
                other => {
                    return Err(TransformError::ParseError { position: tok.pos, message: format!("unexpected token `{other:?}` in `OVER` clause") });
                }
            }
        }
    }

    fn parse_window_frame(&mut self, rows_pos: usize) -> Result<WindowFrame, TransformError> {
        if self.eat_ident_eq("between") {
            let start = self.parse_frame_bound()?;
            self.expect_ident_eq("and", "expected `AND` in `ROWS BETWEEN ... AND ...`")?;
            let end = self.parse_frame_bound()?;
            validate_frame_bounds(&start, &end, rows_pos)?;
            Ok(WindowFrame { start, end })
        } else {
            let start = self.parse_frame_bound()?;
            let end = FrameBound::CurrentRow;
            if matches!(start, FrameBound::Following(_) | FrameBound::UnboundedFollowing) {
                return Err(TransformError::ParseError {
                    position: rows_pos,
                    message: String::from("`ROWS` single frame bound cannot be `FOLLOWING`; use `ROWS BETWEEN ... AND ...`"),
                });
            }
            validate_frame_bounds(&start, &end, rows_pos)?;
            Ok(WindowFrame { start, end })
        }
    }

    fn parse_frame_bound(&mut self) -> Result<FrameBound, TransformError> {
        let pos = self.current_pos();
        if self.eat_ident_eq("unbounded") {
            if self.eat_ident_eq("preceding") {
                Ok(FrameBound::UnboundedPreceding)
            } else if self.eat_ident_eq("following") {
                Ok(FrameBound::UnboundedFollowing)
            } else {
                Err(TransformError::ParseError { position: pos, message: String::from("expected `PRECEDING` or `FOLLOWING` after `UNBOUNDED`") })
            }
        } else if self.eat_ident_eq("current") {
            self.expect_ident_eq("row", "expected `ROW` after `CURRENT` in `ROWS` frame")?;
            Ok(FrameBound::CurrentRow)
        } else {
            let offset = self.parse_expr(5)?;
            validate_static_frame_offset(&offset, pos)?;
            if self.eat_ident_eq("preceding") {
                Ok(FrameBound::Preceding(Box::new(offset)))
            } else if self.eat_ident_eq("following") {
                Ok(FrameBound::Following(Box::new(offset)))
            } else {
                Err(TransformError::ParseError {
                    position: self.current_pos(),
                    message: String::from("expected `PRECEDING` or `FOLLOWING` after frame offset"),
                })
            }
        }
    }
}

fn static_literal_offset(expr: &Expr) -> Option<f64> {
    match expr {
        Expr::Number(v) => Some(*v),
        Expr::Unary { op: UnaryOp::Neg, expr } => static_literal_offset(expr).map(|v| -v),
        Expr::Unary { op: UnaryOp::Pos, expr } => static_literal_offset(expr),
        _ => None,
    }
}

fn validate_static_frame_offset(expr: &Expr, pos: usize) -> Result<(), TransformError> {
    if let Some(v) = static_literal_offset(expr)
        && (v < 0.0 || v.fract() != 0.0 || !v.is_finite())
    {
        return Err(TransformError::ParseError { position: pos, message: format!("`ROWS` frame offset must be a non-negative integer, got {v}") });
    }
    Ok(())
}

fn validate_frame_bounds(start: &FrameBound, end: &FrameBound, pos: usize) -> Result<(), TransformError> {
    match (start, end) {
        (FrameBound::UnboundedFollowing, _) => {
            Err(TransformError::ParseError { position: pos, message: String::from("`UNBOUNDED FOLLOWING` cannot be used as frame start") })
        }
        (_, FrameBound::UnboundedPreceding) => {
            Err(TransformError::ParseError { position: pos, message: String::from("`UNBOUNDED PRECEDING` cannot be used as frame end") })
        }
        (FrameBound::CurrentRow, FrameBound::Preceding(_)) => {
            Err(TransformError::ParseError { position: pos, message: String::from("frame start `CURRENT ROW` cannot follow frame end `PRECEDING`") })
        }
        (FrameBound::Following(_), FrameBound::Preceding(_) | FrameBound::CurrentRow) => Err(TransformError::ParseError {
            position: pos,
            message: String::from("frame start `FOLLOWING` cannot precede `PRECEDING` or `CURRENT ROW`"),
        }),
        (FrameBound::Preceding(a), FrameBound::Preceding(b)) => {
            if let (Some(va), Some(vb)) = (static_literal_offset(a), static_literal_offset(b))
                && va < vb
            {
                return Err(TransformError::ParseError {
                    position: pos,
                    message: format!("invalid `ROWS` frame: `{va} PRECEDING` cannot follow `{vb} PRECEDING`"),
                });
            }
            Ok(())
        }
        (FrameBound::Following(a), FrameBound::Following(b)) => {
            if let (Some(va), Some(vb)) = (static_literal_offset(a), static_literal_offset(b))
                && va > vb
            {
                return Err(TransformError::ParseError {
                    position: pos,
                    message: format!("invalid `ROWS` frame: `{va} FOLLOWING` cannot precede `{vb} FOLLOWING`"),
                });
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn normalize_special_call(pos: usize, name: String, args: Vec<Expr>) -> Result<Expr, TransformError> {
    if name.eq_ignore_ascii_case("col") {
        if args.len() != 1 {
            return Err(TransformError::ArityMismatch { function: String::from("col"), expected: String::from("1"), actual: args.len() });
        }
        return match &args[0] {
            Expr::Text(col_name) => Ok(Expr::Var(VarRef::ColumnByName(col_name.clone()))),
            Expr::Number(v) if *v >= 0.0 && v.fract() == 0.0 && *v <= (usize::MAX as f64) =>
            {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                Ok(Expr::Var(VarRef::ColumnByIndex(*v as usize)))
            }
            _ => Err(TransformError::ParseError {
                position: pos,
                message: String::from("`col(...)` requires a string column name or non-negative integer index"),
            }),
        };
    }
    if name.eq_ignore_ascii_case("info") {
        if args.len() != 1 {
            return Err(TransformError::ArityMismatch { function: String::from("Info"), expected: String::from("1"), actual: args.len() });
        }
        return match &args[0] {
            Expr::Text(key) => Ok(Expr::Var(VarRef::InfoConstant(key.clone()))),
            Expr::Var(VarRef::Ident(key) | VarRef::Bracketed(key)) => Ok(Expr::Var(VarRef::InfoConstant(key.clone()))),
            _ => {
                Err(TransformError::ParseError { position: pos, message: String::from("`Info(...)` requires a string or identifier constant name") })
            }
        };
    }
    Ok(Expr::Call { name, args })
}
