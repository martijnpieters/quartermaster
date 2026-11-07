//! The Issues pane and view: an item's issue drafts with the GitHub repo their numbers link into
//! (`Gui#issues_payload`), and every item's drafts in one list (`IssueIndex`).

use serde_json::{json, Value};

use crate::data_root;
use crate::issue_drafts;
use crate::work_item::{WorkItem, STATES};

fn slug_shaped(slug: &str) -> bool {
    slug.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()) && slug.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

fn repo(project: Option<&str>) -> Option<String> {
    crate::project_map::root_for(project).and_then(|root| crate::branch_diff::github_repo(&root))
}

/// `GET /api/issues/<slug>`; `None` for what only Ruby reads exactly.
pub fn payload(slug: &str) -> Option<Value> {
    let item = match WorkItem::find(slug) {
        Ok(i) => i,
        Err(e) => {
            if e.class != "ArgumentError" {
                data_root::log_error("gui-issues", e.class, &e.message);
            }
            return Some(json!({"ok": false, "error": e.message}));
        }
    };
    if !slug_shaped(&item.slug) {
        return Some(json!({"ok": false, "error": format!("'{}' is not a work item slug", item.slug)}));
    }
    let mut out = json!({"ok": true, "slug": item.slug, "repo": repo(item.project().as_deref())});
    for (k, v) in issue_drafts::read(&item.slug)?.as_object().cloned().unwrap_or_default() {
        out[k] = v;
    }
    Some(out)
}

/// `GET /api/issues`: one entry per work item whose issues directory holds a draft or a broken file.
pub fn index() -> Option<Value> {
    let Ok(dirs) = std::fs::read_dir(data_root::root().join("issues")) else { return Some(json!({"ok": true, "items": []})) };
    let mut slugs: Vec<String> = vec![];
    for d in dirs.filter_map(|e| e.ok()) {
        let name = d.file_name().into_string().ok()?;
        if d.path().is_dir() && slug_shaped(&name) {
            slugs.push(name);
        }
    }
    slugs.sort();
    let mut repos: Vec<(Option<String>, Option<String>)> = vec![];
    let mut entries: Vec<(usize, f64, i64, String, Value)> = vec![];
    for slug in slugs {
        let files = issue_drafts::files(&slug)?;
        if files.is_empty() {
            continue;
        }
        let Ok(item) = WorkItem::find(&slug) else { continue };
        let data = issue_drafts::read(&slug)?;
        let project = item.project();
        let repo = match repos.iter().find(|(p, _)| *p == project) {
            Some((_, r)) => r.clone(),
            None => {
                let r = repo(project.as_deref());
                repos.push((project.clone(), r.clone()));
                r
            }
        };
        let changed = files
            .iter()
            .filter_map(|f| std::fs::metadata(issue_drafts::dir(&slug).join(f)).and_then(|m| m.modified()).ok())
            .map(crate::clock::epoch_f64)
            .fold(None, |best: Option<f64>, t| Some(best.map_or(t, |b| b.max(t))))
            .unwrap_or(0.0);
        let issues: Vec<Value> = data["issues"].as_array().cloned().unwrap_or_default().into_iter().map(|i| json!({"file": i["file"], "title": i["title"], "kind": i["kind"], "number": i["number"], "created": i["created"]})).collect();
        let state = item.state();
        let rank = state.as_deref().and_then(|s| STATES.iter().position(|x| *x == s)).unwrap_or(STATES.len());
        let stamp = chrono::DateTime::<chrono::Utc>::from_timestamp(changed.floor() as i64, 0).map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)).unwrap_or_default();
        entries.push((rank, -changed, 0, slug.clone(), json!({"slug": slug, "title": item.title(), "state": state, "repo": repo, "changed_at": stamp, "issues": issues, "errors": data["errors"]})));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)).then(a.3.cmp(&b.3)));
    Some(json!({"ok": true, "items": entries.into_iter().map(|e| e.4).collect::<Vec<_>>()}))
}

/// `Gui#workspace_payload`: where "work on this" would land, for the side panel: the target, the item's
/// dev server and IDE, the branch the chat runs on and the default selection. Creates nothing.
pub fn workspace(slug: &str, query: Option<&str>, params: &serde_json::Map<String, Value>) -> Option<Value> {
    let _ = query;
    let failed = |class: &str, message: String| {
        if class != "ArgumentError" {
            data_root::log_error("gui-workspace", class, &message);
        }
        Some(json!({"ok": false, "error": message}))
    };
    let mut item = match WorkItem::find(slug) {
        Ok(i) => i,
        Err(e) => return failed(e.class, e.message),
    };
    let picked = params.get("branch").and_then(Value::as_str).unwrap_or("").to_string();
    let chat_branch = crate::branch_diff::chat_branch(&item);
    let empty = serde_json::Map::new();
    let chains = crate::gh_stack::chains(&item, &empty, &empty);
    let in_chain = chat_branch.as_ref().is_some_and(|b| chains.iter().any(|c| c["layers"].as_array().is_some_and(|l| l.iter().any(|x| x["branch"] == b.as_str()))));
    let default = if in_chain { chat_branch.clone() } else { chains.first().and_then(|c| c["layers"].as_array()).and_then(|l| l.last()).and_then(|l| l["branch"].as_str()).map(String::from) };
    let branch = if picked.is_empty() { default.clone() } else { Some(picked) };
    let target = match crate::workspace::resolve(&mut item, None, branch.as_deref()) {
        Ok(t) => t,
        Err(m) => return Some(json!({"ok": false, "error": m})),
    };
    let config = crate::project_config::ProjectConfig::current();
    let project = item.project();
    let server = config.settings(project.as_deref()).and_then(|s| s.dev_server.as_ref());
    let short = |p: &str| crate::cli_format::short_path(Some(p));
    let alternatives: Vec<Value> = target.alternatives.iter().map(|(b, p)| json!({"branch": b, "path": p, "short": short(p)})).collect();
    Some(json!({
        "ok": true,
        "devServer": server.map(|s| json!({"command": s.command, "modes": s.modes, "mode": item.pf_mode()})),
        "ide": config.ide(project.as_deref()).map(|i| json!({"label": i.label, "command": i.command})),
        "root": crate::project_map::root_for(project.as_deref()),
        "target": {"path": target.path, "short": short(&target.path), "branch": target.branch, "create": target.create, "warning": target.warning, "alternatives": alternatives},
        "chatBranch": chat_branch, "defaultBranch": default
    }))
}
