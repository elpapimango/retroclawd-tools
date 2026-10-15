//! Expressions: numbers, symbols, the current address and the usual operators.
//!
//! `$FF`, `0xFF`, `0FFh`, `%1010`, `0b1010`, decimal and `'A'` are numbers; `$` or `*` alone is the
//! address of the current instruction; `<x` / `>x` are the low / high byte; the binary operators are
//! the C ones (`* / % + - << >> < <= > >= == != & ^ | && ||`) with C precedence; `~`, `!` and unary
//! `-`/`+` complete the set. Values are 64-bit signed; fitting them into a byte or word is the
//! instruction's business.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    Num(i64),
    Sym(String),
    Pc,
    Unary(char, Box<Expr>),
    Binary(Op, Box<Expr>, Box<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Mul,
    Div,
    Rem,
    Add,
    Sub,
    Shl,
    Shr,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Xor,
    Or,
    LAnd,
    LOr,
}

/// Where symbol values come from while evaluating.
pub trait Scope {
    /// The value of a symbol, `None` if it is not defined (yet).
    fn symbol(&self, name: &str) -> Option<i64>;
    /// The address of the current instruction.
    fn pc(&self) -> i64;
}

/// How an evaluation went.
#[derive(Debug, PartialEq, Eq)]
pub enum EvalError {
    Undefined(String),
    Other(String),
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalError::Undefined(name) => write!(f, "undefined symbol \"{name}\""),
            EvalError::Other(message) => f.write_str(message),
        }
    }
}

impl Expr {
    pub fn eval(&self, scope: &dyn Scope) -> Result<i64, EvalError> {
        Ok(match self {
            Expr::Num(n) => *n,
            Expr::Pc => scope.pc(),
            Expr::Sym(name) => scope
                .symbol(name)
                .ok_or_else(|| EvalError::Undefined(name.clone()))?,
            Expr::Unary(op, inner) => {
                let v = inner.eval(scope)?;
                match op {
                    '-' => v.wrapping_neg(),
                    '+' => v,
                    '~' => !v,
                    '!' => (v == 0) as i64,
                    '<' => v & 0xFF,
                    '>' => (v >> 8) & 0xFF,
                    _ => unreachable!("parser only builds these"),
                }
            }
            Expr::Binary(op, l, r) => {
                // `&&` and `||` do not evaluate (or complain about) the side they skip.
                if *op == Op::LAnd {
                    let a = l.eval(scope)?;
                    return Ok((a != 0 && r.eval(scope)? != 0) as i64);
                }
                if *op == Op::LOr {
                    let a = l.eval(scope)?;
                    return Ok((a != 0 || r.eval(scope)? != 0) as i64);
                }
                let a = l.eval(scope)?;
                let b = r.eval(scope)?;
                match op {
                    Op::Mul => a.wrapping_mul(b),
                    Op::Div | Op::Rem if b == 0 => {
                        return Err(EvalError::Other("division by zero".to_string()));
                    }
                    Op::Div => a.wrapping_div(b),
                    Op::Rem => a.wrapping_rem(b),
                    Op::Add => a.wrapping_add(b),
                    Op::Sub => a.wrapping_sub(b),
                    Op::Shl | Op::Shr if !(0..64).contains(&b) => {
                        return Err(EvalError::Other(format!("shift by {b}")));
                    }
                    Op::Shl => a.wrapping_shl(b as u32),
                    Op::Shr => a >> b,
                    Op::Lt => (a < b) as i64,
                    Op::Le => (a <= b) as i64,
                    Op::Gt => (a > b) as i64,
                    Op::Ge => (a >= b) as i64,
                    Op::Eq => (a == b) as i64,
                    Op::Ne => (a != b) as i64,
                    Op::And => a & b,
                    Op::Xor => a ^ b,
                    Op::Or => a | b,
                    Op::LAnd | Op::LOr => unreachable!("handled above"),
                }
            }
        })
    }

    /// Every symbol name the expression mentions.
    pub fn symbols(&self, out: &mut Vec<String>) {
        match self {
            Expr::Sym(name) => out.push(name.clone()),
            Expr::Unary(_, e) => e.symbols(out),
            Expr::Binary(_, l, r) => {
                l.symbols(out);
                r.symbols(out);
            }
            Expr::Num(_) | Expr::Pc => {}
        }
    }
}

/// Parses a complete expression; trailing text is an error.
pub fn parse(text: &str) -> Result<Expr, String> {
    let mut p = Parser {
        chars: text.chars().collect(),
        pos: 0,
    };
    p.skip_space();
    if p.pos >= p.chars.len() {
        return Err("missing expression".to_string());
    }
    let expr = p.binary(0)?;
    p.skip_space();
    if p.pos < p.chars.len() {
        let rest: String = p.chars[p.pos..].iter().collect();
        return Err(format!("unexpected \"{rest}\" in expression"));
    }
    Ok(expr)
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

/// Binary operators by precedence level, lowest first.
const LEVELS: &[&[(&str, Op)]] = &[
    &[("||", Op::LOr)],
    &[("&&", Op::LAnd)],
    &[("|", Op::Or)],
    &[("^", Op::Xor)],
    &[("&", Op::And)],
    &[("==", Op::Eq), ("!=", Op::Ne)],
    &[("<=", Op::Le), (">=", Op::Ge), ("<", Op::Lt), (">", Op::Gt)],
    &[("<<", Op::Shl), (">>", Op::Shr)],
    &[("+", Op::Add), ("-", Op::Sub)],
    &[("*", Op::Mul), ("/", Op::Div), ("%", Op::Rem)],
];

impl Parser {
    fn skip_space(&mut self) {
        while self.pos < self.chars.len() && self.chars[self.pos].is_whitespace() {
            self.pos += 1;
        }
    }

    fn peek_is(&self, text: &str) -> bool {
        text.chars()
            .enumerate()
            .all(|(i, c)| self.chars.get(self.pos + i) == Some(&c))
    }

    fn binary(&mut self, level: usize) -> Result<Expr, String> {
        if level == LEVELS.len() {
            return self.unary();
        }
        let mut left = self.binary(level + 1)?;
        loop {
            self.skip_space();
            // Longest operator first within the level ("<=" before "<"), and never "<" when the
            // text is really "<<" (that belongs to a higher-precedence level and was consumed already).
            let Some((text, op)) = LEVELS[level].iter().find(|(t, _)| self.peek_is(t)) else {
                return Ok(left);
            };
            if self.confusable(level, text) {
                return Ok(left);
            }
            self.pos += text.chars().count();
            let right = self.binary(level + 1)?;
            left = Expr::Binary(*op, Box::new(left), Box::new(right));
        }
    }

    /// `|` must not eat the first half of `||`, `&` of `&&`, `<` of `<<` / `<=`, `>` of `>>` / `>=`.
    fn confusable(&self, level: usize, text: &str) -> bool {
        let next = self.chars.get(self.pos + text.chars().count()).copied();
        match (level, text) {
            (2, "|") => next == Some('|'),
            (4, "&") => next == Some('&'),
            (6, "<") => matches!(next, Some('<') | Some('=')),
            (6, ">") => matches!(next, Some('>') | Some('=')),
            _ => false,
        }
    }

    fn unary(&mut self) -> Result<Expr, String> {
        self.skip_space();
        match self.chars.get(self.pos).copied() {
            Some(c @ ('-' | '+' | '~' | '!' | '<' | '>')) => {
                // `%` is a unary-position binary literal, handled in `primary`.
                self.pos += 1;
                Ok(Expr::Unary(c, Box::new(self.unary()?)))
            }
            _ => self.primary(),
        }
    }

    fn primary(&mut self) -> Result<Expr, String> {
        self.skip_space();
        let Some(c) = self.chars.get(self.pos).copied() else {
            return Err("expression ends too early".to_string());
        };
        match c {
            '(' => {
                self.pos += 1;
                let inner = self.binary(0)?;
                self.skip_space();
                if self.chars.get(self.pos) != Some(&')') {
                    return Err("missing \")\" in expression".to_string());
                }
                self.pos += 1;
                Ok(inner)
            }
            '*' => {
                self.pos += 1;
                Ok(Expr::Pc)
            }
            '\'' => self.char_literal(),
            '$' => {
                let hex: String = self.chars[self.pos + 1..]
                    .iter()
                    .take_while(|c| c.is_ascii_hexdigit() || **c == '_')
                    .collect();
                if hex.is_empty() {
                    self.pos += 1;
                    return Ok(Expr::Pc);
                }
                self.pos += 1 + hex.chars().count();
                self.guard_word_end()?;
                i64::from_str_radix(&hex.replace('_', ""), 16)
                    .map(Expr::Num)
                    .map_err(|_| format!("hex number ${hex} is too large"))
            }
            '%' => {
                let bin: String = self.chars[self.pos + 1..]
                    .iter()
                    .take_while(|c| matches!(c, '0' | '1' | '_'))
                    .collect();
                if bin.is_empty() {
                    return Err("\"%\" needs binary digits here".to_string());
                }
                self.pos += 1 + bin.chars().count();
                i64::from_str_radix(&bin.replace('_', ""), 2)
                    .map(Expr::Num)
                    .map_err(|_| format!("binary number %{bin} is too large"))
            }
            c if c.is_ascii_digit() => self.number(),
            c if is_symbol_start(c) => {
                let start = self.pos;
                while self.pos < self.chars.len() && is_symbol_char(self.chars[self.pos]) {
                    self.pos += 1;
                }
                Ok(Expr::Sym(self.chars[start..self.pos].iter().collect()))
            }
            other => Err(format!("unexpected \"{other}\" in expression")),
        }
    }

    /// A number must not run into letters (`12abc`).
    fn guard_word_end(&self) -> Result<(), String> {
        match self.chars.get(self.pos) {
            Some(c) if c.is_ascii_alphanumeric() || *c == '_' => {
                Err(format!("bad number: unexpected \"{c}\""))
            }
            _ => Ok(()),
        }
    }

    fn number(&mut self) -> Result<Expr, String> {
        let start = self.pos;
        while self.pos < self.chars.len()
            && (self.chars[self.pos].is_ascii_alphanumeric() || self.chars[self.pos] == '_')
        {
            self.pos += 1;
        }
        let word: String = self.chars[start..self.pos].iter().collect();
        let lower = word.to_ascii_lowercase().replace('_', "");
        let (digits, radix) = if let Some(h) = lower.strip_prefix("0x") {
            (h.to_string(), 16)
        } else if let Some(b) = lower.strip_prefix("0b") {
            (b.to_string(), 2)
        } else if lower.len() > 1 && lower.ends_with('h') {
            (lower[..lower.len() - 1].to_string(), 16)
        } else {
            (lower, 10)
        };
        i64::from_str_radix(&digits, radix)
            .map(Expr::Num)
            .map_err(|_| format!("\"{word}\" is not a valid number"))
    }

    fn char_literal(&mut self) -> Result<Expr, String> {
        // pos is at the opening quote.
        let mut i = self.pos + 1;
        let c = *self.chars.get(i).ok_or("unterminated character literal")?;
        let value = if c == '\\' {
            i += 1;
            let e = *self.chars.get(i).ok_or("unterminated character literal")?;
            match e {
                'n' => b'\n',
                'r' => b'\r',
                't' => b'\t',
                '0' => 0,
                '\\' => b'\\',
                '\'' => b'\'',
                '"' => b'"',
                other => return Err(format!("unknown escape \\{other}")),
            }
        } else if c.is_ascii() {
            c as u8
        } else {
            return Err("character literals are ASCII".to_string());
        };
        i += 1;
        if self.chars.get(i) != Some(&'\'') {
            return Err("unterminated character literal".to_string());
        }
        self.pos = i + 1;
        Ok(Expr::Num(value as i64))
    }
}

pub fn is_symbol_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '@' || c == '.'
}

pub fn is_symbol_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '@' || c == '.'
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Env(HashMap<&'static str, i64>, i64);
    impl Scope for Env {
        fn symbol(&self, name: &str) -> Option<i64> {
            self.0.get(name).copied()
        }
        fn pc(&self) -> i64 {
            self.1
        }
    }

    fn eval(text: &str) -> Result<i64, String> {
        let env = Env(HashMap::from([("start", 0xC000), ("n", 5)]), 0x1000);
        parse(text)?.eval(&env).map_err(|e| e.to_string())
    }

    #[test]
    fn number_notations() {
        assert_eq!(eval("$FF"), Ok(255));
        assert_eq!(eval("0xff"), Ok(255));
        assert_eq!(eval("0FFh"), Ok(255));
        assert_eq!(eval("38h"), Ok(0x38));
        assert_eq!(eval("%1010_0101"), Ok(0xA5));
        assert_eq!(eval("0b11"), Ok(3));
        assert_eq!(eval("1234"), Ok(1234));
        assert_eq!(eval("'A'"), Ok(65));
        assert_eq!(eval("'\\n'"), Ok(10));
        assert_eq!(
            eval("'A"),
            Err("unterminated character literal".to_string())
        );
        assert_eq!(eval("'''"), Ok(39));
    }

    #[test]
    fn the_current_address() {
        assert_eq!(eval("*"), Ok(0x1000));
        assert_eq!(eval("$"), Ok(0x1000));
        assert_eq!(eval("* + 2"), Ok(0x1002));
        assert_eq!(eval("$ - start"), Ok(0x1000 - 0xC000));
        assert_eq!(eval("2 * 3"), Ok(6));
    }

    #[test]
    fn precedence_and_grouping() {
        assert_eq!(eval("1 + 2 * 3"), Ok(7));
        assert_eq!(eval("(1 + 2) * 3"), Ok(9));
        assert_eq!(eval("1 << 4 | 3"), Ok(19));
        assert_eq!(eval("10 - 4 - 3"), Ok(3));
        assert_eq!(eval("7 % 4 + 1"), Ok(4));
        assert_eq!(eval("1 + 1 == 2 && 3 > 2"), Ok(1));
        assert_eq!(eval("0 || 0"), Ok(0));
        assert_eq!(eval("6 & 3 ^ 1"), Ok(3));
        assert_eq!(eval("-3 + 5"), Ok(2));
        assert_eq!(eval("~0 & $FF"), Ok(255));
        assert_eq!(eval("!5"), Ok(0));
    }

    #[test]
    fn low_and_high_byte() {
        assert_eq!(eval("<start"), Ok(0));
        assert_eq!(eval(">start"), Ok(0xC0));
        assert_eq!(eval("<$1234 + 1"), Ok(0x35));
        assert_eq!(eval(">($1234 + $100)"), Ok(0x13));
    }

    #[test]
    fn symbols_and_errors() {
        assert_eq!(eval("start + n"), Ok(0xC005));
        assert_eq!(
            eval("missing + 1"),
            Err("undefined symbol \"missing\"".to_string())
        );
        assert_eq!(eval("1 / 0"), Err("division by zero".to_string()));
        assert!(eval("1 +").is_err());
        assert!(eval("(1").is_err());
        assert!(eval("1 2").is_err());
        assert!(eval("12abc").is_err());
        assert!(eval("").is_err());
        // The short-circuit operators do not evaluate the skipped side.
        assert_eq!(eval("1 || missing"), Ok(1));
        assert_eq!(eval("0 && missing"), Ok(0));
    }

    #[test]
    fn local_and_dotted_symbol_names() {
        let e = parse("@loop + .x").unwrap();
        let mut names = Vec::new();
        e.symbols(&mut names);
        assert_eq!(names, ["@loop", ".x"]);
    }
}
