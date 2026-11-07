//! What qm knows about a pull request from the PRs view (`PrItems`): the project whose checkout it belongs to,
//! the work item holding its branch, its review item.

use std::sync::Mutex;

use serde_json::Value;

use crate::work_item::WorkItem;

static REPO_PROJECTS: Mutex<Option<Vec<(String, String)>>> = Mutex::new(None);

/// "Owner/Name" → the configured project whose main checkout's origin it is; worked out once.
pub fn project_for_repo(repo: &str) -> Option<String> {
    let mut cache = REPO_PROJECTS.lock().unwrap_or_else(|p| p.into_inner());
    let map = cache.get_or_insert_with(|| {
        let config = crate::project_config::ProjectConfig::current();
        let mut out: Vec<(String, String)> = vec![];
        for project in config.project_names() {
            let root = crate::project_map::root_for(Some(&project));
            let repo = root.and_then(|r| crate::branch_diff::github_repo(&r)).unwrap_or_default().to_lowercase();
            out.retain(|(k, _)| *k != repo);
            out.push((repo, project));
        }
        out
    });
    map.iter().find(|(k, _)| *k == repo.to_lowercase()).map(|(_, p)| p.clone())
}

fn field<'a>(pr: &'a Value, key: &str) -> &'a str {
    pr.get(key).and_then(Value::as_str).unwrap_or("")
}

pub fn pr_label(pr: &Value) -> String {
    let repo = field(pr, "repo");
    let number = match pr.get("number") {
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    };
    format!("{}#{number}", repo.rsplit('/').next().unwrap_or(""))
}

pub fn pr_summary(pr: &Value) -> String {
    format!("{} by @{}: `{}` → `{}`", field(pr, "url"), field(pr, "author"), field(pr, "head"), field(pr, "base"))
}

/// The item holding the PR's branch, an unresolved one first.
pub fn pr_item<'a>(pr: &Value, project: Option<&str>, items: &'a [WorkItem]) -> Option<&'a WorkItem> {
    let mut best: Option<&WorkItem> = None;
    for item in items.iter().filter(|i| i.project().as_deref() == project && i.branches().iter().any(|b| b == field(pr, "head"))) {
        let rank = |i: &WorkItem| (i.state().as_deref() == Some("resolved")) as u8;
        if best.is_none_or(|b| rank(item) < rank(b)) {
            best = Some(item);
        }
    }
    best
}

/// Found by slug prefix, not title: the PR's title may change on GitHub.
pub fn review_item<'a>(pr: &Value, items: &'a [WorkItem]) -> Option<&'a WorkItem> {
    let prefix = WorkItem::slugify(&format!("Review {}", pr_label(pr)));
    items.iter().find(|i| i.slug == prefix || i.slug.starts_with(&format!("{prefix}-")))
}

/// A Review item's first prompt: the review skill on the PR, and where its findings go.
pub fn review_kickoff(skill: &str, pr: &Value, slug: &str) -> String {
    format!("{skill} {}\n\nRecord each finding as a qm draft with `qm draft add {slug} …` (see `qm draft add --help`) instead of posting to GitHub; the user curates and posts them.", field(pr, "url"))
}
