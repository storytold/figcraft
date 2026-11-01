//! Renaming several layers at once (`object.batchRename`, the Rename layers dialog): a match
//! (plain text, or a regular expression) whose occurrences are replaced, and/or a pattern for the
//! whole name. Both understand the codes: `$&` the current name (the matched text when matching),
//! `` $` `` / `$'` the text before / after the match, `$1`…`$9` a regular expression's groups,
//! `$n` a number counting up and `$N` one counting down (`$nnn` pads it to three digits), `$$` a
//! dollar sign.

use figcraft_doc::{Document, NodeId};

/// Longest match pattern accepted (characters).
const MAX_FIND: usize = 1_000;
/// Longest name a rename produces (characters).
const MAX_NAME: usize = 10_000;
/// Widest zero padding of a number code.
const MAX_PAD: usize = 12;

enum Find {
    Text(String),
    Regex(regex::Regex),
}

/// One batch rename's settings.
pub struct BatchRename {
    find: Option<Find>,
    replace: String,
    pattern: Option<String>,
    /// Where `$n` starts and `$N` stops.
    start: u64,
}

/// What the codes of one replacement stand for.
struct Ctx<'a> {
    matched: &'a str,
    before: &'a str,
    after: &'a str,
    groups: Option<&'a regex::Captures<'a>>,
    up: u64,
    down: u64,
}

impl BatchRename {
    /// `find` (empty: none) as text or, with `regex`, a regular expression, replaced by `replace`;
    /// then `pattern` for the whole name; numbers from `start` (default 1).
    pub fn new(find: Option<&str>, regex: bool, replace: &str, pattern: Option<&str>, start: Option<f64>) -> Result<Self, String> {
        let find = match find.filter(|f| !f.is_empty()) {
            None => None,
            Some(f) if f.chars().count() > MAX_FIND => return Err(format!("the match is longer than {MAX_FIND} characters")),
            Some(f) if regex => {
                Some(Find::Regex(regex::RegexBuilder::new(f).size_limit(1 << 20).build().map_err(|e| format!("invalid regular expression: {e}"))?))
            }
            Some(f) => Some(Find::Text(f.to_string())),
        };
        let start = start.filter(|s| s.is_finite()).map_or(1, |s| s.clamp(0.0, 1e12).round() as u64);
        Ok(Self { find, replace: replace.to_string(), pattern: pattern.map(str::to_string), start })
    }

    /// The new name of layer `k` of `n` (stacking order, bottom first), now `name`.
    pub fn apply(&self, name: &str, k: usize, n: usize) -> String {
        let up = self.start.saturating_add(k as u64);
        let down = self.start.saturating_add(n.saturating_sub(k + 1) as u64);
        let mut out = match &self.find {
            None => name.to_string(),
            Some(Find::Text(f)) => {
                let mut out = String::new();
                let mut last = 0;
                for (i, _) in name.match_indices(f.as_str()) {
                    let end = i + f.len();
                    out.push_str(name.get(last..i).unwrap_or_default());
                    let (before, after) = (name.get(..i).unwrap_or_default(), name.get(end..).unwrap_or_default());
                    expand(&self.replace, &Ctx { matched: f, before, after, groups: None, up, down }, &mut out);
                    last = end;
                }
                out.push_str(name.get(last..).unwrap_or_default());
                out
            }
            Some(Find::Regex(re)) => {
                let mut out = String::new();
                let mut last = 0;
                for caps in re.captures_iter(name) {
                    let Some(m) = caps.get(0) else { continue };
                    out.push_str(name.get(last..m.start()).unwrap_or_default());
                    let (before, after) = (name.get(..m.start()).unwrap_or_default(), name.get(m.end()..).unwrap_or_default());
                    expand(&self.replace, &Ctx { matched: m.as_str(), before, after, groups: Some(&caps), up, down }, &mut out);
                    if out.len() >= MAX_NAME * 4 {
                        break;
                    }
                    last = m.end();
                }
                out.push_str(name.get(last..).unwrap_or_default());
                out
            }
        };
        if let Some(p) = &self.pattern {
            let mut whole = String::new();
            expand(p, &Ctx { matched: &out, before: "", after: "", groups: None, up, down }, &mut whole);
            out = whole;
        }
        match out.char_indices().nth(MAX_NAME) {
            Some((cut, _)) => out.get(..cut).unwrap_or_default().to_string(),
            None => out,
        }
    }

    /// The new names of `ids` (taken in stacking order, bottom first).
    pub fn plan(&self, doc: &Document, ids: &[NodeId]) -> Vec<(NodeId, String)> {
        let ids = crate::cmd::edit::z_sorted(doc, ids);
        let n = ids.len();
        ids.iter().enumerate().filter_map(|(k, i)| doc.node(*i).map(|node| (*i, self.apply(&node.name, k, n)))).collect()
    }
}

/// Append `template` with its codes filled in from `c`.
fn expand(template: &str, c: &Ctx, out: &mut String) {
    let mut it = template.chars().peekable();
    while let Some(ch) = it.next() {
        if out.len() >= MAX_NAME * 4 {
            break;
        }
        if ch != '$' {
            out.push(ch);
            continue;
        }
        match it.peek().copied() {
            Some('&') => out.push_str(c.matched),
            Some('`') => out.push_str(c.before),
            Some('\'') => out.push_str(c.after),
            Some('$') => out.push('$'),
            Some(d @ ('n' | 'N')) => {
                let mut width = 0;
                while it.peek() == Some(&d) {
                    it.next();
                    width += 1;
                }
                let v = if d == 'n' { c.up } else { c.down };
                out.push_str(&format!("{v:0w$}", w = width.min(MAX_PAD)));
                continue;
            }
            Some(d @ '1'..='9') => match c.groups.filter(|g| (d as usize - '0' as usize) < g.len()) {
                Some(g) => out.push_str(g.get(d as usize - '0' as usize).map_or("", |m| m.as_str())),
                // Not a group of this match: the code stays as typed.
                None => {
                    out.push('$');
                    out.push(d);
                }
            },
            _ => {
                out.push('$');
                continue;
            }
        }
        it.next();
    }
}
