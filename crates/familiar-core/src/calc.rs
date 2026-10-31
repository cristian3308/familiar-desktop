//! The launcher's inline calculator: `= 2*(3+4)` or just `12/5`.
//!
//! A small recursive-descent parser over `+ - * / % ^`, parentheses, unary
//! minus, decimal points or commas, the constants `pi` and `e`, and a few
//! functions (`sqrt`, `abs`, `round`, `floor`, `ceil`, `ln`, `log`, `sin`,
//! `cos`, `tan`). Nothing is ever executed.

/// Evaluates `query` when it reads as arithmetic. Plain words ("chrome")
/// and bare numbers ("2026") are not calculations and return `None`.
pub fn evaluate(query: &str) -> Option<f64> {
    let q = query.trim();
    let (expr, forced) = match q.strip_prefix('=') {
        Some(rest) => (rest.trim(), true),
        None => (q, false),
    };
    if expr.is_empty() {
        return None;
    }
    if !forced {
        // Without "=", require digits and an operator so app names never
        // turn into calculations.
        let has_digit = expr.chars().any(|c| c.is_ascii_digit());
        let has_op = expr.chars().skip(1).any(|c| "+-*/%^x×÷(".contains(c));
        let only_math = expr.chars().all(|c| c.is_ascii_digit() || " .,+-*/%^()x×÷".contains(c));
        if !(has_digit && has_op && only_math) {
            return None;
        }
    }
    let mut p = Parser { s: normalise(expr), i: 0 };
    let v = p.expr()?;
    p.skip_ws();
    (p.i == p.s.len() && v.is_finite()).then_some(v)
}

/// Formats a result: integers without decimals, otherwise up to 10
/// significant decimals without trailing zeros.
pub fn format(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    let s = format!("{v:.10}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn normalise(expr: &str) -> Vec<char> {
    expr.chars()
        .map(|c| match c {
            ',' => '.',
            'x' | '×' => '*',
            '÷' => '/',
            c => c.to_ascii_lowercase(),
        })
        .collect()
}

struct Parser {
    s: Vec<char>,
    i: usize,
}

impl Parser {
    fn skip_ws(&mut self) {
        while self.s.get(self.i).is_some_and(|c| c.is_whitespace()) {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.skip_ws();
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    // expr := term (('+' | '-') term)*
    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        loop {
            if self.eat('+') {
                v += self.term()?;
            } else if self.eat('-') {
                v -= self.term()?;
            } else {
                return Some(v);
            }
        }
    }

    // term := power (('*' | '/' | '%') power)*
    fn term(&mut self) -> Option<f64> {
        let mut v = self.power()?;
        loop {
            if self.eat('*') {
                v *= self.power()?;
            } else if self.eat('/') {
                let d = self.power()?;
                if d == 0.0 {
                    return None;
                }
                v /= d;
            } else if self.eat('%') {
                let d = self.power()?;
                if d == 0.0 {
                    return None;
                }
                v %= d;
            } else {
                return Some(v);
            }
        }
    }

    // power := unary ('^' power)?   (right-associative)
    fn power(&mut self) -> Option<f64> {
        let base = self.unary()?;
        if self.eat('^') { Some(base.powf(self.power()?)) } else { Some(base) }
    }

    // unary := '-' unary | '+' unary | atom
    fn unary(&mut self) -> Option<f64> {
        if self.eat('-') {
            return Some(-self.unary()?);
        }
        if self.eat('+') {
            return self.unary();
        }
        self.atom()
    }

    // atom := number | '(' expr ')' | name | name '(' expr ')'
    fn atom(&mut self) -> Option<f64> {
        if self.eat('(') {
            let v = self.expr()?;
            return self.eat(')').then_some(v);
        }
        let c = self.peek()?;
        if c.is_ascii_digit() || c == '.' {
            let start = self.i;
            while self.s.get(self.i).is_some_and(|c| c.is_ascii_digit() || *c == '.') {
                self.i += 1;
            }
            return self.s[start..self.i].iter().collect::<String>().parse().ok();
        }
        if c.is_ascii_alphabetic() {
            let start = self.i;
            while self.s.get(self.i).is_some_and(|c| c.is_ascii_alphabetic()) {
                self.i += 1;
            }
            let name: String = self.s[start..self.i].iter().collect();
            match name.as_str() {
                "pi" => return Some(std::f64::consts::PI),
                "e" => return Some(std::f64::consts::E),
                _ => {}
            }
            if !self.eat('(') {
                return None;
            }
            let arg = self.expr()?;
            if !self.eat(')') {
                return None;
            }
            return match name.as_str() {
                "sqrt" if arg >= 0.0 => Some(arg.sqrt()),
                "abs" => Some(arg.abs()),
                "round" => Some(arg.round()),
                "floor" => Some(arg.floor()),
                "ceil" => Some(arg.ceil()),
                "ln" if arg > 0.0 => Some(arg.ln()),
                "log" if arg > 0.0 => Some(arg.log10()),
                "sin" => Some(arg.sin()),
                "cos" => Some(arg.cos()),
                "tan" => Some(arg.tan()),
                _ => None,
            };
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{evaluate, format};

    #[test]
    fn arithmetic() {
        assert_eq!(evaluate("2+3*4"), Some(14.0));
        assert_eq!(evaluate("(2+3)*4"), Some(20.0));
        assert_eq!(evaluate("2^3^2"), Some(512.0));
        assert_eq!(evaluate("-3 + 5"), Some(2.0));
        assert_eq!(evaluate("10 % 4"), Some(2.0));
        assert_eq!(evaluate("7 / 2"), Some(3.5));
    }

    #[test]
    fn locale_and_symbols() {
        assert_eq!(evaluate("1,5 x 2"), Some(3.0));
        assert_eq!(evaluate("9 ÷ 3"), Some(3.0));
    }

    #[test]
    fn functions_need_equals() {
        assert_eq!(evaluate("= sqrt(16) + abs(-2)"), Some(6.0));
        assert_eq!(evaluate("sqrt(16)"), None);
        assert!((evaluate("= 2*pi").unwrap() - std::f64::consts::TAU).abs() < 1e-12);
    }

    #[test]
    fn not_a_calculation() {
        assert_eq!(evaluate("chrome"), None);
        assert_eq!(evaluate("2026"), None);
        assert_eq!(evaluate("-5"), None);
        assert_eq!(evaluate("7-zip"), None);
        assert_eq!(evaluate("= 1/0"), None);
        assert_eq!(evaluate("= (2"), None);
        assert_eq!(evaluate("="), None);
    }

    #[test]
    fn forced_bare_number() {
        assert_eq!(evaluate("= 42"), Some(42.0));
    }

    #[test]
    fn formatting() {
        assert_eq!(format(14.0), "14");
        assert_eq!(format(3.5), "3.5");
        assert_eq!(format(1.0 / 3.0), "0.3333333333");
        assert_eq!(format(-2.0), "-2");
    }
}
