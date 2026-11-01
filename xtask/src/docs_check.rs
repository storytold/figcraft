//! Progress docs damaged by merges: a table row that appears twice, or a status line (`**Change:**
//! kind (…; …)`) that lists the same item twice. Both happen when a merge keeps both sides of a
//! conflict; they are never wanted.

use std::collections::HashMap;
use std::path::Path;

/// Checks `ROADMAP.md` and `docs/*.md` under `root`; prints each repeat and returns whether there
/// were none.
pub fn check(root: &Path) -> bool {
    let mut files = vec![root.join("ROADMAP.md")];
    if let Ok(rd) = std::fs::read_dir(root.join("docs")) {
        let mut docs: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "md")).collect();
        docs.sort();
        files.extend(docs);
    }
    let mut found = Vec::new();
    for f in &files {
        let Ok(text) = std::fs::read_to_string(f) else { continue };
        let name = f.strip_prefix(root).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
        found.extend(repeats(&text).into_iter().map(|r| format!("{name}:{r}")));
    }
    if found.is_empty() {
        eprintln!("docs: ok");
        return true;
    }
    eprintln!("docs: repeated rows or status items (left by a merge?):\n  {}", found.join("\n  "));
    false
}

/// `line: what` for each table row that repeats an earlier one (header rows apart: tables share
/// them) and each status-line item listed twice.
fn repeats(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let separator = |l: &str| l.starts_with('|') && l.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '));
    let mut out = Vec::new();
    let mut rows: HashMap<&str, usize> = HashMap::new();
    for (i, l) in lines.iter().enumerate() {
        let header = lines.get(i + 1).is_some_and(|n| separator(n));
        if l.starts_with('|') && !separator(l) && !header {
            match rows.get(l) {
                Some(first) => out.push(format!("{}: the table row of line {first} again", i + 1)),
                None => {
                    rows.insert(l, i + 1);
                }
            }
        }
        if let Some(body) = status_items(l) {
            let mut seen = Vec::new();
            for item in body {
                if seen.contains(&item) {
                    out.push(format!("{}: status item listed twice: {}", i + 1, item.chars().take(100).collect::<String>()));
                } else {
                    seen.push(item);
                }
            }
        }
    }
    out
}

/// The items of a status line's `**Change:** kind (…)`, split at `; ` outside brackets. `None`
/// for other lines, or when the brackets never close.
fn status_items(line: &str) -> Option<Vec<&str>> {
    let open = line.find("**Change:**").and_then(|i| line[i..].find('(').map(|o| i + o))?;
    let mut depth = 0usize;
    let mut items = Vec::new();
    let mut start = open + 1;
    for (j, c) in line.char_indices().skip_while(|(j, _)| *j < open) {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    items.push(&line[start..j]);
                    return Some(items);
                }
            }
            ';' if depth == 1 && line[j..].starts_with("; ") => {
                items.push(&line[start..j]);
                start = j + 2;
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::repeats;

    #[test]
    fn repeated_rows_and_status_items_are_found_and_headers_are_not() {
        let doc = "**Change:** minor (a (x; y); b; a (x; y)) · **Target:** t\n\n\
                   | Item | Status |\n|---|---|\n| one | done |\n| two | done |\n\n\
                   | Item | Status |\n|---|---|\n| one | done |\n";
        let found = repeats(doc);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[0].starts_with("1: status item listed twice: a (x; y)"), "{found:?}");
        assert!(found[1].starts_with("10: the table row of line 5 again"), "{found:?}");
        // Distinct items, and a parenthesis that never closes, are left alone.
        assert!(repeats("**Change:** minor (a; b (c; c)) · x\n| a | b |\n| a | c |\n").is_empty());
        assert!(repeats("**Change:** minor (a; a (never closed\n").is_empty());
    }
}
