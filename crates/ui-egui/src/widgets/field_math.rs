//! What is typed into a number field: a number, arithmetic (`+ - * /` and brackets, with units
//! like `px`, `°` or `%` after numbers), or an edit relative to the value there now (`+10`, `*2`,
//! `/2`, and `50%` where the field isn't a percentage itself).

/// An edit relative to the value a field has: what `+10`, `*2`, `/4` or `50%` mean. On a field
/// showing mixed values it applies to each layer's own value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rel {
    Add(f64),
    Mul(f64),
    Div(f64),
}

impl Rel {
    /// The value `cur` becomes (none when it isn't a number any more: a division by zero).
    pub fn apply(self, cur: f64) -> Option<f64> {
        let v = match self {
            Rel::Add(b) => cur + b,
            Rel::Mul(b) => cur * b,
            Rel::Div(b) if b != 0.0 => cur / b,
            Rel::Div(_) => return None,
        };
        v.is_finite().then_some(v)
    }
}

/// What a field's text means.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Typed {
    Value(f64),
    Rel(Rel),
}

/// Text longer than this isn't a number someone typed.
const MAX_LEN: usize = 256;
/// Brackets nest this deep at most.
const MAX_DEPTH: usize = 32;

struct Parser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    /// `%` after a number is its unit (a percentage field); otherwise it isn't part of arithmetic.
    percent_unit: bool,
}

impl Parser<'_> {
    fn space(&mut self) {
        while self.chars.next_if(|c| c.is_whitespace()).is_some() {}
    }

    fn eat(&mut self, c: char) -> bool {
        self.space();
        self.chars.next_if_eq(&c).is_some()
    }

    /// Sums and differences of products.
    fn sum(&mut self, depth: usize) -> Option<f64> {
        let mut v = self.product(depth)?;
        loop {
            if self.eat('+') {
                v += self.product(depth)?;
            } else if self.eat('-') {
                v -= self.product(depth)?;
            } else {
                return v.is_finite().then_some(v);
            }
        }
    }

    fn product(&mut self, depth: usize) -> Option<f64> {
        let mut v = self.factor(depth)?;
        loop {
            if self.eat('*') {
                v *= self.factor(depth)?;
            } else if self.eat('/') {
                let d = self.factor(depth)?;
                if d == 0.0 {
                    return None;
                }
                v /= d;
            } else {
                return Some(v);
            }
        }
    }

    /// A number with its unit, a bracketed sum, or either negated.
    fn factor(&mut self, depth: usize) -> Option<f64> {
        if depth > MAX_DEPTH {
            return None;
        }
        if self.eat('-') {
            return self.factor(depth + 1).map(|v| -v);
        }
        if self.eat('(') {
            let v = self.sum(depth + 1)?;
            return self.eat(')').then_some(v);
        }
        let mut digits = String::new();
        while let Some(c) = self.chars.next_if(|c| c.is_ascii_digit() || matches!(c, '.' | ',')) {
            // A decimal comma is a decimal point.
            digits.push(if c == ',' { '.' } else { c });
        }
        let v = digits.parse::<f64>().ok().filter(|v| v.is_finite())?;
        // The unit: letters (px, pt, ms, x), a degree sign, and a percent sign where that is one.
        // It may follow spaces ("3 px").
        let percent_unit = self.percent_unit;
        let is_unit = |c: &char| c.is_ascii_alphabetic() || *c == '°' || (*c == '%' && percent_unit);
        let mut ahead = self.chars.clone();
        while ahead.next_if(|c| c.is_whitespace()).is_some() {}
        if ahead.peek().is_some_and(is_unit) {
            self.space();
        }
        while self.chars.next_if(is_unit).is_some() {}
        Some(v)
    }

    fn end(&mut self) -> bool {
        self.space();
        self.chars.peek().is_none()
    }
}

/// What `text` typed into a number field means. `percent_unit`: the field shows a percentage, so a
/// `%` after a number is just its unit; elsewhere a `%` at the end makes the edit relative
/// (`50%`: half the value, `+10%`: a tenth more, `-10%`: a tenth less).
pub fn parse(text: &str, percent_unit: bool) -> Option<Typed> {
    let text = text.trim();
    if text.is_empty() || text.len() > MAX_LEN {
        return None;
    }
    let percent = (!percent_unit).then(|| text.strip_suffix('%')).flatten();
    let body = percent.unwrap_or(text).trim();
    // What it starts with decides whether it is relative ("-4" stays the number minus four).
    let (lead, rest) = match body.chars().next() {
        Some(c @ ('+' | '*' | '/')) => (Some(c), body.get(c.len_utf8()..).unwrap_or("")),
        _ => (None, body),
    };
    let mut p = Parser { chars: rest.chars().peekable(), percent_unit };
    let v = p.sum(0).filter(|_| p.end())?;
    let rel = match (percent.is_some(), lead) {
        (false, None) => return Some(Typed::Value(v)),
        (false, Some('+')) => Rel::Add(v),
        (false, Some('*')) => Rel::Mul(v),
        (false, Some(_)) => Rel::Div(v),
        // "50%" of the value, "+10%" more (a typed minus is in `v`: "-10%" is a tenth less).
        (true, None) if body.starts_with('-') => Rel::Mul(1.0 + v / 100.0),
        (true, None) => Rel::Mul(v / 100.0),
        (true, Some('+')) => Rel::Mul(1.0 + v / 100.0),
        (true, Some('*')) => Rel::Mul(v / 100.0),
        (true, Some(_)) => Rel::Div(v / 100.0),
    };
    Some(Typed::Rel(rel))
}

/// The number `text` gives a field whose value is `cur` (none: no single value), with every unit
/// read as a unit (`"50%"` is 50). Without a value, `+4` is 4 and other relative edits are nothing.
pub fn eval(text: &str, cur: Option<f64>) -> Option<f64> {
    match (parse(text, true)?, cur) {
        (Typed::Value(v), _) => Some(v),
        (Typed::Rel(r), Some(c)) => r.apply(c),
        (Typed::Rel(Rel::Add(v)), None) => Some(v),
        (Typed::Rel(_), None) => None,
    }
}

/// `v` moved `steps` steps of `step` from `from`, without the dust of repeated additions
/// (0.1 + 0.2).
pub fn stepped(from: f64, steps: f64, step: f64) -> f64 {
    let v = from + steps * step;
    if !v.is_finite() {
        return from;
    }
    let rounded = (v * 1e6).round() / 1e6;
    // Past ~1.8e302 the scaling overflows; such a value has no dust to remove.
    if rounded.is_finite() { rounded } else { v }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_after_a_space() {
        assert_eq!(parse("3px", true), Some(Typed::Value(3.0)));
        assert_eq!(parse("3 px", true), Some(Typed::Value(3.0)));
        assert_eq!(parse("10 °", true), Some(Typed::Value(10.0)));
        assert_eq!(parse("50 %", true), Some(Typed::Value(50.0)));
        assert_eq!(parse("3 px + 2 px", true), Some(Typed::Value(5.0)));
    }

    #[test]
    fn stepped_keeps_a_huge_finite_value_finite() {
        assert_eq!(stepped(1e305, 1.0, 1.0), 1e305);
        assert!(stepped(f64::MAX / 2.0, 0.0, 1.0).is_finite());
    }
}
