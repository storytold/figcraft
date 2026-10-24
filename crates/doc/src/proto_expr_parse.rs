//! Prototype expressions as text: reading what people type in the interaction editor
//! (`count + 1`, `score >= 10 and not done`, `"Hi " + name`) and checking the type an expression
//! gives before it is saved.
//!
//! Variables are written by name (a name with spaces or symbols reads as typed: the longest name
//! that fits wins; `{name}` is always a variable). A variable followed by a mode in brackets
//! (`accent[Dark]`, `{Brand color}["High contrast"]`) reads its value in that mode of its
//! collection; `mode(Theme)` is the name of the mode a collection is in where the expression runs
//! (`mode(Theme) == "Dark"`). Literals: numbers, `"text"` or `'text'`, `true` / `false`,
//! `#RRGGBB` colours. Operators, loosest first: `or` (`||`), `and` (`&&`), `not`
//! (`!`), comparisons (`==` or `=`, `!=`, `<`, `<=`, `>`, `>=`), `+` / `-`, `*` / `/`, unary `-`.
//! [`Expr::display`] writes what this reads back.

use super::{Expr, ExprOp, MAX_EXPR_DEPTH};
use crate::{VariableType, VariableValue, Variables};

/// Longest expression text read (characters).
pub const MAX_EXPR_TEXT: usize = 4_000;

/// Why a text isn't an expression: a message for people and where it went wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExprError {
    pub message: String,
    /// A character offset into the text.
    pub at: usize,
}

impl std::fmt::Display for ExprError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

type Parsed = Result<Expr, ExprError>;

impl Expr {
    /// Read an expression typed as text, variables by name (or id) from `vars`.
    pub fn parse(text: &str, vars: &Variables) -> Parsed {
        let s: Vec<char> = text.chars().collect();
        if s.len() > MAX_EXPR_TEXT {
            return Err(ExprError { message: format!("Keep expressions under {MAX_EXPR_TEXT} characters"), at: 0 });
        }
        // Longest names first, so "count total" wins over "count".
        let mut names: Vec<(Vec<char>, String)> = vars
            .variables
            .iter()
            .flat_map(|v| [(v.name.chars().collect::<Vec<char>>(), v.id.clone()), (v.id.chars().collect(), v.id.clone())])
            .filter(|(n, _)| !n.is_empty())
            .collect();
        names.sort_by_key(|(n, _)| std::cmp::Reverse(n.len()));
        let mut p = Parser { s, i: 0, names, vars };
        p.ws();
        if p.peek(0).is_none() {
            return Err(ExprError { message: "Type an expression".into(), at: 0 });
        }
        let e = p.or(0)?;
        p.ws();
        match p.peek(0) {
            None => Ok(e),
            Some(_) => Err(p.unexpected()),
        }
    }

    /// The type the expression gives, or why it can't give one (a missing variable, operands the
    /// operator can't take).
    pub fn type_of(&self, vars: &Variables) -> Result<VariableType, String> {
        self.type_at(vars, 0)
    }

    /// A condition must give true or false.
    pub fn check_condition(&self, vars: &Variables) -> Result<(), String> {
        match self.type_of(vars)? {
            VariableType::Boolean => Ok(()),
            t => Err(format!("A condition must be true or false, not {}", noun(t))),
        }
    }

    /// A value for a variable of `kind` (text variables take numbers and booleans as text).
    pub fn check_value(&self, vars: &Variables, kind: VariableType) -> Result<(), String> {
        let t = self.type_of(vars)?;
        let fits = t == kind || (kind == VariableType::String && matches!(t, VariableType::Number | VariableType::Boolean));
        if fits { Ok(()) } else { Err(format!("This gives {}, but the variable holds {}", noun(t), noun(kind))) }
    }

    fn type_at(&self, vars: &Variables, depth: usize) -> Result<VariableType, String> {
        use VariableType as T;
        if depth > MAX_EXPR_DEPTH {
            return Err("The expression is nested too deeply".into());
        }
        let (op, args) = match self {
            Expr::Value(VariableValue::Alias(id)) => return vars.get(id).map(|v| v.kind).ok_or_else(|| format!("No variable `{id}`")),
            Expr::Value(VariableValue::Number(n)) if !n.is_finite() => return Err("Numbers must be finite".into()),
            Expr::Value(v) => return v.type_of().ok_or_else(|| "Not a value".to_string()),
            Expr::Mode { collection } => {
                return vars.collection(collection).map(|_| T::String).ok_or_else(|| format!("No variable collection `{collection}`"));
            }
            Expr::InMode { variable, mode } => {
                let v = vars.get(variable).ok_or_else(|| format!("No variable `{variable}`"))?;
                let c = vars.collection(&v.collection).ok_or_else(|| format!("No variable collection `{}`", v.collection))?;
                return c.find_mode(mode).map(|_| v.kind).ok_or_else(|| format!("{} has no mode `{mode}`", c.name));
            }
            Expr::Op { op, args } => (*op, args),
        };
        let ts = args.iter().map(|a| a.type_at(vars, depth + 1)).collect::<Result<Vec<T>, String>>()?;
        let all = |k: T| !ts.is_empty() && ts.iter().all(|t| *t == k);
        let sym = op.symbol();
        match op {
            ExprOp::Add if ts.contains(&T::String) => {
                if ts.contains(&T::Color) {
                    Err("Colours can't be joined to text".into())
                } else {
                    Ok(T::String)
                }
            }
            ExprOp::Add | ExprOp::Subtract | ExprOp::Multiply | ExprOp::Divide => {
                if all(T::Number) {
                    Ok(T::Number)
                } else if op == ExprOp::Add {
                    Err("`+` adds numbers or joins text".into())
                } else {
                    Err(format!("`{sym}` needs numbers"))
                }
            }
            ExprOp::Equals | ExprOp::NotEqual => match ts.as_slice() {
                [] | [_] => Err(format!("`{sym}` compares two values")),
                [first, rest @ ..] => match rest.iter().find(|t| *t != first) {
                    None => Ok(T::Boolean),
                    Some(other) => Err(format!("Can't compare {} with {}", noun(*first), noun(*other))),
                },
            },
            ExprOp::Less | ExprOp::LessOrEqual | ExprOp::Greater | ExprOp::GreaterOrEqual => match ts.as_slice() {
                [T::Number, T::Number] | [T::String, T::String] => Ok(T::Boolean),
                _ => Err(format!("`{sym}` compares two numbers or two texts")),
            },
            ExprOp::And | ExprOp::Or => {
                if all(T::Boolean) {
                    Ok(T::Boolean)
                } else {
                    Err(format!("`{sym}` needs true or false on both sides"))
                }
            }
            ExprOp::Not => match ts.as_slice() {
                [T::Boolean] => Ok(T::Boolean),
                _ => Err("`not` needs true or false".into()),
            },
            ExprOp::Negate => match ts.as_slice() {
                [T::Number] => Ok(T::Number),
                [T::Boolean] => Ok(T::Boolean),
                _ => Err("`-` needs a number".into()),
            },
        }
    }
}

/// A type as a noun in messages.
pub fn noun(t: VariableType) -> &'static str {
    match t {
        VariableType::Number => "a number",
        VariableType::String => "text",
        VariableType::Boolean => "true or false",
        VariableType::Color => "a colour",
    }
}

struct Parser<'a> {
    s: Vec<char>,
    i: usize,
    /// Variable names and ids (as characters) with the id they stand for, longest first.
    names: Vec<(Vec<char>, String)>,
    vars: &'a Variables,
}

fn ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Parser<'_> {
    fn peek(&self, k: usize) -> Option<char> {
        self.s.get(self.i + k).copied()
    }

    fn ws(&mut self) {
        while self.peek(0).is_some_and(char::is_whitespace) {
            self.i += 1;
        }
    }

    fn err(&self, message: impl Into<String>) -> ExprError {
        ExprError { message: message.into(), at: self.i }
    }

    fn deeper(&self, depth: usize) -> Result<usize, ExprError> {
        // Each pair of brackets goes through every precedence level (about eight calls).
        if depth >= MAX_EXPR_DEPTH * 8 { Err(self.err("The expression is nested too deeply")) } else { Ok(depth + 1) }
    }

    /// `word` (any case) here and not running on into a longer word.
    fn at_keyword(&self, word: &str) -> bool {
        let n = word.chars().count();
        word.chars().enumerate().all(|(k, c)| self.peek(k).is_some_and(|x| x.to_lowercase().eq(c.to_lowercase()))) && !self.peek(n).is_some_and(ident)
    }

    /// The first character from `k` characters on that isn't whitespace.
    fn next_after(&self, k: usize) -> Option<char> {
        self.s.iter().skip(self.i + k).copied().find(|c| !c.is_whitespace())
    }

    fn at_symbol(&self, sym: &str) -> bool {
        sym.chars().enumerate().all(|(k, c)| self.peek(k) == Some(c))
    }

    /// Consume one of `words` (keywords) or `symbols`.
    fn eat_any(&mut self, words: &[&str], symbols: &[&str]) -> bool {
        self.ws();
        let hit = words.iter().find(|w| self.at_keyword(w)).or_else(|| symbols.iter().find(|s| self.at_symbol(s)));
        match hit {
            Some(h) => {
                self.i += h.chars().count();
                true
            }
            None => false,
        }
    }

    /// The variable whose name (or id) starts here: (id, length in characters).
    fn variable_here(&self) -> Option<(String, usize)> {
        self.names.iter().find_map(|(name, id)| {
            let n = name.len();
            let same = name.iter().enumerate().all(|(k, c)| self.peek(k) == Some(*c));
            // "count" doesn't match the start of "counter".
            let runs_on = name.last().is_some_and(|c| ident(*c)) && self.peek(n).is_some_and(ident);
            (same && !runs_on).then(|| (id.clone(), n))
        })
    }

    /// A chain of one loose operator (`a or b or c` → one `or` of three).
    fn chain(&mut self, depth: usize, op: ExprOp, words: &[&str], symbols: &[&str], next: fn(&mut Self, usize) -> Parsed) -> Parsed {
        let d = self.deeper(depth)?;
        let first = next(self, d)?;
        let mut args = vec![first];
        while self.eat_any(words, symbols) {
            args.push(next(self, d)?);
        }
        Ok(match <[Expr; 1]>::try_from(args) {
            Ok([one]) => one,
            Err(args) => Expr::op(op, args),
        })
    }

    fn or(&mut self, depth: usize) -> Parsed {
        self.chain(depth, ExprOp::Or, &["or"], &["||"], Self::and)
    }

    fn and(&mut self, depth: usize) -> Parsed {
        self.chain(depth, ExprOp::And, &["and"], &["&&"], Self::not)
    }

    fn not(&mut self, depth: usize) -> Parsed {
        let d = self.deeper(depth)?;
        self.ws();
        let bang = self.peek(0) == Some('!') && self.peek(1) != Some('=');
        if self.variable_here().is_none() && (self.at_keyword("not") || bang) {
            self.i += if bang { 1 } else { 3 };
            return Ok(Expr::op(ExprOp::Not, vec![self.not(d)?]));
        }
        self.compare(d)
    }

    fn compare_op(&mut self) -> Option<ExprOp> {
        self.ws();
        let ops = [
            ("==", ExprOp::Equals),
            ("!=", ExprOp::NotEqual),
            ("<=", ExprOp::LessOrEqual),
            (">=", ExprOp::GreaterOrEqual),
            ("<", ExprOp::Less),
            (">", ExprOp::Greater),
            ("=", ExprOp::Equals),
        ];
        let (sym, op) = ops.iter().find(|(s, _)| self.at_symbol(s))?;
        self.i += sym.len();
        Some(*op)
    }

    fn compare(&mut self, depth: usize) -> Parsed {
        let d = self.deeper(depth)?;
        let left = self.sum(d)?;
        let Some(op) = self.compare_op() else { return Ok(left) };
        let mut args = vec![left, self.sum(d)?];
        loop {
            let at = self.i;
            match self.compare_op() {
                None => break,
                // `a == b == c`: all equal.
                Some(o) if o == op && matches!(op, ExprOp::Equals | ExprOp::NotEqual) => args.push(self.sum(d)?),
                Some(_) => {
                    self.i = at;
                    self.ws();
                    return Err(self.err("Compare two values at a time: join comparisons with `and` or `or`"));
                }
            }
        }
        Ok(Expr::op(op, args))
    }

    /// Left-to-right operators of one level: a run of the same one is one n-ary operation
    /// (`a - b - c`), a change of operator nests (`a + b - c` → `(a + b) - c`).
    fn level(&mut self, depth: usize, ops: &[(char, ExprOp)], next: fn(&mut Self, usize) -> Parsed) -> Parsed {
        let d = self.deeper(depth)?;
        let mut cur = next(self, d)?;
        let mut cur_op: Option<ExprOp> = None;
        loop {
            self.ws();
            let Some((_, op)) = ops.iter().find(|(c, _)| self.peek(0) == Some(*c)) else { break };
            self.i += 1;
            let rhs = next(self, d)?;
            match &mut cur {
                Expr::Op { args, .. } if cur_op == Some(*op) => args.push(rhs),
                _ => {
                    cur = Expr::op(*op, vec![cur, rhs]);
                    cur_op = Some(*op);
                }
            }
        }
        Ok(cur)
    }

    fn sum(&mut self, depth: usize) -> Parsed {
        self.level(depth, &[('+', ExprOp::Add), ('-', ExprOp::Subtract)], Self::product)
    }

    fn product(&mut self, depth: usize) -> Parsed {
        self.level(depth, &[('*', ExprOp::Multiply), ('/', ExprOp::Divide)], Self::unary)
    }

    fn unary(&mut self, depth: usize) -> Parsed {
        let d = self.deeper(depth)?;
        self.ws();
        if self.peek(0) == Some('-') {
            self.i += 1;
            self.ws();
            // `-3` is a negative number; `-x` negates.
            if self.peek(0).is_some_and(|c| c.is_ascii_digit() || c == '.') {
                return match self.number()? {
                    Expr::Value(VariableValue::Number(n)) => Ok(Expr::Value(VariableValue::Number(-n))),
                    other => Ok(Expr::op(ExprOp::Negate, vec![other])),
                };
            }
            return Ok(Expr::op(ExprOp::Negate, vec![self.unary(d)?]));
        }
        self.primary(d)
    }

    /// Append the characters `ok` accepts from here to `out`.
    fn take(&mut self, out: &mut String, ok: impl Fn(char) -> bool) {
        while let Some(c) = self.peek(0).filter(|c| ok(*c)) {
            out.push(c);
            self.i += 1;
        }
    }

    fn number(&mut self) -> Parsed {
        let start = self.i;
        let mut text = String::new();
        self.take(&mut text, |c| c.is_ascii_digit() || c == '.');
        // An exponent: 1e3, 2.5E-4.
        let exp_digit = |k: usize| self.peek(k).is_some_and(|c| c.is_ascii_digit());
        if matches!(self.peek(0), Some('e' | 'E')) && (exp_digit(1) || (matches!(self.peek(1), Some('+' | '-')) && exp_digit(2))) {
            text.push('e');
            self.i += 1;
            if let Some(sign) = self.peek(0).filter(|c| *c == '+' || *c == '-') {
                text.push(sign);
                self.i += 1;
            }
            self.take(&mut text, |c| c.is_ascii_digit());
        }
        match text.parse::<f64>() {
            Ok(n) if n.is_finite() => Ok(Expr::Value(VariableValue::Number(n))),
            _ => Err(ExprError { message: format!("`{text}` isn't a number"), at: start }),
        }
    }

    fn string(&mut self, quote: char) -> Parsed {
        self.text(quote).map(|t| Expr::Value(VariableValue::String(t)))
    }

    /// The text of a literal in `quote`s starting here.
    fn text(&mut self, quote: char) -> Result<String, ExprError> {
        let start = self.i;
        self.i += 1;
        let mut out = String::new();
        loop {
            match self.peek(0) {
                None => return Err(ExprError { message: format!("Close the text with {quote}"), at: start }),
                Some(c) if c == quote => {
                    self.i += 1;
                    return Ok(out);
                }
                Some('\\') => {
                    match self.peek(1) {
                        Some('n') => out.push('\n'),
                        Some(c) => out.push(c),
                        None => return Err(ExprError { message: format!("Close the text with {quote}"), at: start }),
                    }
                    self.i += 2;
                }
                Some(c) => {
                    out.push(c);
                    self.i += 1;
                }
            }
        }
    }

    fn primary(&mut self, depth: usize) -> Parsed {
        let d = self.deeper(depth)?;
        self.ws();
        let Some(c) = self.peek(0) else { return Err(self.err("Something is missing at the end")) };
        match c {
            '(' => {
                self.i += 1;
                let e = self.or(d)?;
                self.ws();
                if self.peek(0) != Some(')') {
                    return Err(self.err("Add a closing )"));
                }
                self.i += 1;
                Ok(e)
            }
            '{' => {
                let start = self.i;
                let close = (self.i + 1..self.s.len()).find(|k| self.s.get(*k) == Some(&'}'));
                let Some(end) = close else { return Err(self.err("Add a closing }")) };
                let name: String = self.s.get(start + 1..end).unwrap_or_default().iter().collect();
                self.i = end + 1;
                match self.vars.get(name.trim()) {
                    Some(v) => self.in_mode(v.id.clone()),
                    None => Err(ExprError { message: format!("No variable `{}`", name.trim()), at: start }),
                }
            }
            '"' | '\'' => self.string(c),
            '#' => {
                let start = self.i;
                self.i += 1;
                let mut hex = String::new();
                while let Some(h) = self.peek(0).filter(char::is_ascii_hexdigit) {
                    hex.push(h);
                    self.i += 1;
                }
                match figcraft_color::Color::from_hex(&hex).filter(|_| matches!(hex.len(), 3 | 4 | 6 | 8)) {
                    Some(col) => Ok(Expr::Value(VariableValue::Color(col))),
                    None => Err(ExprError { message: format!("`#{hex}` isn't a colour (use #RRGGBB)"), at: start }),
                }
            }
            c if c.is_ascii_digit() || c == '.' => self.number(),
            ')' => Err(self.err("Unexpected )")),
            _ => {
                // `mode(…)`, also when a variable is named "mode" (a variable can't take brackets).
                if self.at_keyword("mode") && self.next_after(4) == Some('(') {
                    return self.mode_of();
                }
                let var = self.variable_here();
                // A bare `true`/`false` is the literal, also when a variable has that name (it reads
                // as `{true}`); longer names that start with it ("true value") stay variables.
                for (word, v) in [("true", true), ("false", false)] {
                    if self.at_keyword(word) && var.as_ref().is_none_or(|(_, n)| *n <= word.len()) {
                        self.i += word.len();
                        return Ok(Expr::Value(VariableValue::Boolean(v)));
                    }
                }
                if let Some((id, n)) = var {
                    self.i += n;
                    return self.in_mode(id);
                }
                let word: String = self.s.iter().skip(self.i).take_while(|c| ident(**c)).collect();
                if word.is_empty() { Err(self.err(format!("Unexpected {c}"))) } else { Err(self.err(format!("No variable `{word}`"))) }
            }
        }
    }

    /// `mode(collection)` starting here: the mode the collection is in.
    fn mode_of(&mut self) -> Parsed {
        let start = self.i;
        while self.peek(0).is_some_and(|c| c != '(') {
            self.i += 1;
        }
        self.i += 1;
        let name = self.argument(')', "collection")?;
        match self.vars.collection(&name) {
            Some(c) => Ok(Expr::Mode { collection: c.id.clone() }),
            None => Err(ExprError { message: format!("No variable collection `{name}`"), at: start }),
        }
    }

    /// Variable `var`, or with a mode in brackets after it its value in that mode of its collection.
    fn in_mode(&mut self, var: String) -> Parsed {
        if self.next_after(0) != Some('[') {
            return Ok(Expr::var(var));
        }
        self.ws();
        self.i += 1;
        let start = self.i;
        let name = self.argument(']', "mode")?;
        let coll = self.vars.get(&var).and_then(|v| self.vars.collection(&v.collection));
        match coll.and_then(|c| c.find_mode(&name)) {
            Some(m) => Ok(Expr::InMode { variable: var, mode: m.id.clone() }),
            None => {
                let whose = coll.map_or("The variable's collection", |c| c.name.as_str());
                Err(ExprError { message: format!("{whose} has no mode `{name}`"), at: start })
            }
        }
    }

    /// A collection or mode name up to `close` (quoted, or as typed with the spaces around it and
    /// any `{…}` dropped), then past `close`.
    fn argument(&mut self, close: char, what: &str) -> Result<String, ExprError> {
        self.ws();
        let start = self.i;
        let name = match self.peek(0) {
            Some(q @ ('"' | '\'')) => self.text(q)?,
            _ => {
                let mut raw = String::new();
                self.take(&mut raw, |c| c != close);
                let t = raw.trim();
                t.strip_prefix('{').and_then(|x| x.strip_suffix('}')).unwrap_or(t).trim().to_string()
            }
        };
        self.ws();
        if self.peek(0) != Some(close) {
            return Err(self.err(format!("Add a closing {close}")));
        }
        self.i += 1;
        if name.is_empty() {
            return Err(ExprError { message: format!("Name a {what} between the brackets"), at: start });
        }
        Ok(name)
    }

    /// What follows a complete expression that shouldn't.
    fn unexpected(&self) -> ExprError {
        let word: String = self.s.iter().skip(self.i).take_while(|c| ident(**c)).collect();
        match self.peek(0) {
            Some(')') => self.err("Unexpected ): there's no ( to close"),
            _ if !word.is_empty() => self.err(format!("Add an operator before `{word}`")),
            Some(c) => self.err(format!("Unexpected {c}")),
            None => self.err("Something is missing at the end"),
        }
    }
}
