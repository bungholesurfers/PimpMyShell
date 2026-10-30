//! Tiny arithmetic over named counter values:
//!   expr   = term (('+' | '-') term)*
//!   term   = unary (('*' | '/') unary)*
//!   unary  = '-' unary | atom
//!   atom   = number | name | name '(' expr (',' expr)* ')' | '(' expr ')'
//! Functions: min, max, abs.

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum Expr {
    Num(f64),
    Var(String),
    Neg(Box<Expr>),
    Bin(char, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Op(char),
}

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let mut out = Vec::new();
    let mut chars = src.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c.is_ascii_digit() || c == '.' {
            let mut s = String::new();
            while let Some(c) = chars.next_if(|c| c.is_ascii_alphanumeric() || *c == '.') {
                s.push(c);
            }
            // Allow exponents like 1e-3: the '-' after 'e' belongs to the number.
            if s.ends_with(['e', 'E'])
                && let Some(sign) = chars.next_if(|c| *c == '-' || *c == '+') {
                    s.push(sign);
                    while let Some(c) = chars.next_if(char::is_ascii_digit) {
                        s.push(c);
                    }
                }
            out.push(Tok::Num(s.parse().map_err(|_| format!("bad number `{s}`"))?));
        } else if c.is_alphabetic() || c == '_' {
            let mut s = String::new();
            while let Some(c) = chars.next_if(|c| c.is_alphanumeric() || *c == '_') {
                s.push(c);
            }
            out.push(Tok::Ident(s));
        } else if "+-*/(),".contains(c) {
            out.push(Tok::Op(c));
            chars.next();
        } else {
            return Err(format!("unexpected `{c}`"));
        }
    }
    Ok(out)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn eat(&mut self, op: char) -> bool {
        if self.peek() == Some(&Tok::Op(op)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expr(&mut self) -> Result<Expr, String> {
        let mut lhs = self.term()?;
        loop {
            let op = if self.eat('+') { '+' } else if self.eat('-') { '-' } else { return Ok(lhs) };
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(self.term()?));
        }
    }
    fn term(&mut self) -> Result<Expr, String> {
        let mut lhs = self.unary()?;
        loop {
            let op = if self.eat('*') { '*' } else if self.eat('/') { '/' } else { return Ok(lhs) };
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(self.unary()?));
        }
    }
    fn unary(&mut self) -> Result<Expr, String> {
        if self.eat('-') { Ok(Expr::Neg(Box::new(self.unary()?))) } else { self.atom() }
    }
    fn atom(&mut self) -> Result<Expr, String> {
        match self.toks.get(self.pos).cloned() {
            Some(Tok::Num(n)) => {
                self.pos += 1;
                Ok(Expr::Num(n))
            }
            Some(Tok::Ident(name)) => {
                self.pos += 1;
                if !self.eat('(') {
                    return Ok(Expr::Var(name));
                }
                let mut args = vec![self.expr()?];
                while self.eat(',') {
                    args.push(self.expr()?);
                }
                if !self.eat(')') {
                    return Err(format!("missing `)` after arguments to {name}"));
                }
                Ok(Expr::Call(name, args))
            }
            Some(Tok::Op('(')) => {
                self.pos += 1;
                let e = self.expr()?;
                if !self.eat(')') {
                    return Err("missing `)`".into());
                }
                Ok(e)
            }
            other => Err(format!("expected a value, found {other:?}")),
        }
    }
}

pub fn parse(src: &str) -> Result<Expr, String> {
    let mut p = Parser { toks: lex(src)?, pos: 0 };
    let e = p.expr()?;
    match p.peek() {
        None => Ok(e),
        Some(t) => Err(format!("unexpected {t:?} after expression")),
    }
}

impl Expr {
    /// Every variable name referenced, for validating against configured counters up front.
    pub fn vars(&self, out: &mut Vec<String>) {
        match self {
            Expr::Num(_) => {}
            Expr::Var(v) => out.push(v.clone()),
            Expr::Neg(e) => e.vars(out),
            Expr::Bin(_, a, b) => {
                a.vars(out);
                b.vars(out);
            }
            Expr::Call(_, args) => args.iter().for_each(|a| a.vars(out)),
        }
    }

    pub fn eval(&self, env: &HashMap<String, f64>) -> Result<f64, String> {
        Ok(match self {
            Expr::Num(n) => *n,
            Expr::Var(v) => *env.get(v).ok_or_else(|| format!("unknown name `{v}`"))?,
            Expr::Neg(e) => -e.eval(env)?,
            Expr::Bin(op, a, b) => {
                let (a, b) = (a.eval(env)?, b.eval(env)?);
                match op {
                    '+' => a + b,
                    '-' => a - b,
                    '*' => a * b,
                    _ => a / b,
                }
            }
            Expr::Call(f, args) => {
                let vals = args.iter().map(|a| a.eval(env)).collect::<Result<Vec<_>, _>>()?;
                match (f.as_str(), vals.as_slice()) {
                    ("abs", [x]) => x.abs(),
                    ("min", [first, rest @ ..]) => rest.iter().fold(*first, |m, v| m.min(*v)),
                    ("max", [first, rest @ ..]) => rest.iter().fold(*first, |m, v| m.max(*v)),
                    _ => return Err(format!("unknown function {f}/{}", vals.len())),
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(src: &str, env: &[(&str, f64)]) -> f64 {
        let env = env.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        parse(src).unwrap().eval(&env).unwrap()
    }

    #[test]
    fn precedence_and_functions() {
        assert_eq!(eval("1 + 2 * 3", &[]), 7.0);
        assert_eq!(eval("(1 + 2) * 3", &[]), 9.0);
        assert_eq!(eval("-2 * -3", &[]), 6.0);
        assert_eq!(eval("100 * (1 - avail / total)", &[("avail", 25.0), ("total", 100.0)]), 75.0);
        assert_eq!(eval("max(a, b, 3) - min(a, b)", &[("a", 1.0), ("b", 2.0)]), 2.0);
        assert_eq!(eval("abs(x) * 1e-3", &[("x", -2000.0)]), 2.0);
        assert_eq!(eval("8 * net_rx / 1e6", &[("net_rx", 125000.0)]), 1.0);
    }

    #[test]
    fn errors() {
        assert!(parse("1 +").is_err());
        assert!(parse("(1").is_err());
        assert!(parse("1 $ 2").is_err());
        assert!(parse("max(1").is_err());
    }
}
