//! Public GitHub credits for About. Requests run in the background on desktop and in the browser;
//! the embedded contributor snapshot remains available when GitHub cannot be reached.

use std::collections::BTreeMap;
#[cfg(not(test))]
use std::sync::{Arc, Mutex, OnceLock};
#[cfg(not(test))]
use std::time::Duration;

#[cfg(not(test))]
use egui::Context;
use serde_json::Value;

#[cfg(not(test))]
const API: &str = "https://api.github.com/repos/storytold/figcraft";
#[cfg(not(test))]
const PAGE_SIZE: usize = 100;
#[cfg(not(test))]
const MAX_PAGES: usize = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Contributor {
    pub login: String,
    pub commits: u64,
    pub prs: u64,
}

#[derive(Debug)]
pub(crate) struct Snapshot {
    pub contributors: Vec<Contributor>,
    pub total_commits: u64,
    pub prs_complete: bool,
}

#[cfg(not(test))]
#[derive(Clone, Default)]
pub(crate) enum State {
    #[default]
    Idle,
    Loading,
    Ready(Arc<Snapshot>),
    Failed,
}

#[cfg(not(test))]
static STATE: OnceLock<Mutex<State>> = OnceLock::new();

#[cfg(not(test))]
fn cache() -> &'static Mutex<State> {
    STATE.get_or_init(|| Mutex::new(State::Idle))
}

/// Fetch only when the contributor tab is first opened, once per running app. Refresh is explicit.
#[cfg(not(test))]
pub(crate) fn state(ctx: &Context) -> State {
    let Ok(mut guard) = cache().lock() else { return State::Failed };
    let start = matches!(*guard, State::Idle);
    if start {
        *guard = State::Loading;
    }
    let state = guard.clone();
    drop(guard);
    if start {
        fetch_contributors(1, Vec::new(), ctx.clone());
    }
    state
}

#[cfg(not(test))]
pub(crate) fn refresh(ctx: &Context) {
    let Ok(mut guard) = cache().lock() else { return };
    if matches!(*guard, State::Loading) {
        return;
    }
    *guard = State::Loading;
    drop(guard);
    fetch_contributors(1, Vec::new(), ctx.clone());
    ctx.request_repaint();
}

#[cfg(not(test))]
fn publish(ctx: &Context, result: State) {
    if let Ok(mut guard) = cache().lock() {
        *guard = result;
    }
    ctx.request_repaint();
}

#[cfg(not(test))]
fn request(url: String, callback: impl FnOnce(Result<Vec<u8>, ()>) + Send + 'static) {
    let request = ehttp::Request::get(url).with_header("Accept", "application/vnd.github+json").with_timeout(Some(Duration::from_secs(20)));
    #[cfg(not(target_arch = "wasm32"))]
    let request = request.with_header("User-Agent", "FigCraft");
    ehttp::fetch(request, move |response| {
        let bytes = response.ok().filter(|r| r.status == 200).map(|r| r.bytes).ok_or(());
        callback(bytes);
    });
}

#[cfg(not(test))]
fn fetch_contributors(page: usize, mut gathered: Vec<Contributor>, ctx: Context) {
    request(format!("{API}/contributors?per_page={PAGE_SIZE}&page={page}"), move |result| {
        let Some((mut chunk, page_len)) = result.ok().and_then(|bytes| parse_contributors(&bytes).ok()) else {
            publish(&ctx, State::Failed);
            return;
        };
        let more = page_len == PAGE_SIZE;
        gathered.append(&mut chunk);
        if more {
            if page >= MAX_PAGES {
                publish(&ctx, State::Failed);
            } else {
                fetch_contributors(page + 1, gathered, ctx);
            }
        } else {
            fetch_pulls(1, gathered, Vec::new(), ctx);
        }
    });
}

#[cfg(not(test))]
fn fetch_pulls(page: usize, commits: Vec<Contributor>, mut authors: Vec<String>, ctx: Context) {
    request(format!("{API}/pulls?state=closed&per_page={PAGE_SIZE}&page={page}"), move |result| {
        let Some(mut chunk) = result.ok().and_then(|bytes| parse_merged_authors(&bytes).ok()) else {
            publish(&ctx, State::Ready(Arc::new(combine(commits, None))));
            return;
        };
        // This endpoint contains closed but unmerged PRs too; page length comes from the raw list.
        let more = chunk.1 == PAGE_SIZE;
        authors.append(&mut chunk.0);
        if more {
            if page >= MAX_PAGES {
                publish(&ctx, State::Ready(Arc::new(combine(commits, None))));
            } else {
                fetch_pulls(page + 1, commits, authors, ctx);
            }
        } else {
            publish(&ctx, State::Ready(Arc::new(combine(commits, Some(authors)))));
        }
    });
}

fn valid_login(login: &str) -> bool {
    !login.is_empty() && login.len() <= 39 && login.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
}

fn parse_contributors(bytes: &[u8]) -> Result<(Vec<Contributor>, usize), &'static str> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| "invalid JSON")?;
    let Some(rows) = value.as_array() else { return Err("contributors are not an array") };
    let contributors = rows
        .iter()
        .filter_map(|row| {
            let login = row.get("login")?.as_str()?;
            valid_login(login).then(|| Contributor {
                login: login.to_string(),
                commits: row.get("contributions").and_then(Value::as_u64).unwrap_or(0),
                prs: 0,
            })
        })
        .collect();
    Ok((contributors, rows.len()))
}

/// Return merged authors and the size of the unfiltered page for pagination.
fn parse_merged_authors(bytes: &[u8]) -> Result<(Vec<String>, usize), &'static str> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| "invalid JSON")?;
    let Some(rows) = value.as_array() else { return Err("pulls are not an array") };
    let authors = rows
        .iter()
        .filter(|row| row.get("merged_at").is_some_and(|date| !date.is_null()))
        .filter_map(|row| row.get("user")?.get("login")?.as_str())
        .filter(|login| valid_login(login))
        .map(str::to_owned)
        .collect();
    Ok((authors, rows.len()))
}

fn combine(commits: Vec<Contributor>, merged_authors: Option<Vec<String>>) -> Snapshot {
    let total_commits = commits.iter().fold(0_u64, |n, c| n.saturating_add(c.commits));
    let prs_complete = merged_authors.is_some();
    let mut by_login = BTreeMap::<String, Contributor>::new();
    for contributor in commits {
        let key = contributor.login.to_ascii_lowercase();
        by_login.entry(key).and_modify(|old| old.commits = old.commits.saturating_add(contributor.commits)).or_insert(contributor);
    }
    for login in merged_authors.into_iter().flatten() {
        let key = login.to_ascii_lowercase();
        by_login.entry(key).or_insert_with(|| Contributor { login, commits: 0, prs: 0 }).prs += 1;
    }
    Snapshot { contributors: by_login.into_values().collect(), total_commits, prs_complete }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combines_commit_and_pr_only_authors_without_duplicates() {
        let (commits, page_len) =
            parse_contributors(br#"[{"login":"YarilaFox","contributions":18},{"login":"echelon","contributions":34}]"#).unwrap();
        assert_eq!(page_len, 2);
        let (authors, page_len) = parse_merged_authors(
            br#"[{"user":{"login":"yarilafox"},"merged_at":"2026-10-22T00:00:00Z"},{"user":{"login":"OnlyPR"},"merged_at":"2026-10-22T00:00:00Z"},{"user":{"login":"Closed"},"merged_at":null}]"#,
        )
        .unwrap();
        assert_eq!(page_len, 3);
        let result = combine(commits, Some(authors));
        assert_eq!(result.total_commits, 52);
        assert_eq!(result.contributors.len(), 3);
        assert_eq!(result.contributors.iter().find(|c| c.login == "YarilaFox").map(|c| c.prs), Some(1));
        assert_eq!(result.contributors.iter().find(|c| c.login == "OnlyPR").map(|c| (c.commits, c.prs)), Some((0, 1)));
    }

    #[test]
    fn missing_pr_data_keeps_commit_credits() {
        let result = combine(vec![Contributor { login: "one".into(), commits: 4, prs: 0 }], None);
        assert!(!result.prs_complete);
        assert_eq!(result.contributors.len(), 1);
    }
}
