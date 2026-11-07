//! The PRs view (`Gui#prs_payload`): the cached list (`PrInbox`), each PR tied to what qm knows about it —
//! its project (and dev-server command and review skill), the work item holding its branch, the review
//! item, the worktree it is checked out in and whether the dev server serves that.

use serde_json::{json, Value};

use crate::cli_format::short_path;
use crate::pr_items;
use crate::work_item::WorkItem;

/// `GET /api/prs?refresh=1`; `None` for what only Ruby reads exactly.
pub fn payload(params: &serde_json::Map<String, Value>) -> Option<Value> {
    let mut list = crate::pr_inbox::list(params.get("refresh").and_then(Value::as_str) == Some("1"));
    let config = crate::project_config::ProjectConfig::current();
    let items = match WorkItem::all() {
        Ok(i) => i,
        Err(e) => {
            if e.class != "ArgumentError" {
                crate::data_root::log_error("gui-prs", e.class, &e.message);
            }
            return Some(json!({"ok": false, "error": e.message}));
        }
    };
    let status = crate::pf::status(config.dev_server());
    let panes = crate::tmux::Tmux::new().sessions();
    let prerequisites = match crate::pr_prerequisites::all() {
        Ok(p) => crate::pr_deployment::states(&p, true),
        Err(_) => return None,
    };
    let mut worktrees: Vec<(String, Vec<(String, String)>)> = vec![];
    let prs: Vec<Value> = list["prs"].as_array().cloned().unwrap_or_default().into_iter().map(|pr| {
        let project = pr_items::project_for_repo(pr["repo"].as_str().unwrap_or(""));
        let settings = project.as_deref().and_then(|p| config.settings(Some(p)));
        let root = settings.map(|s| s.root.clone());
        let head = pr["head"].as_str().unwrap_or("").to_string();
        let path = root.and_then(|r| {
            if !worktrees.iter().any(|(k, _)| *k == r) {
                let listing = crate::branch_diff::branch_worktrees(&r);
                worktrees.push((r.clone(), listing));
            }
            worktrees.iter().find(|(k, _)| *k == r).and_then(|(_, l)| l.iter().find(|(b, _)| *b == head).map(|(_, p)| p.clone()))
        });
        let item = pr_items::pr_item(&pr, project.as_deref(), &items);
        let label = pr_items::pr_label(&pr);
        let mut out = pr.as_object().cloned().unwrap_or_default();
        out.insert("label".into(), json!(label));
        out.insert("project".into(), json!(project));
        out.insert("devServer".into(), project.as_ref().map(|_| settings.and_then(|s| s.dev_server.as_ref()).map(|d| d.command.clone())).map(|c| json!(c)).unwrap_or(Value::Null));
        out.insert("ide".into(), project.as_deref().and_then(|p| config.ide(Some(p))).map(|i| json!(i.label)).unwrap_or(Value::Null));
        out.insert("reviewSkill".into(), json!(config.review_skill(project.as_deref())));
        out.insert("item".into(), json!(item.map(|i| i.slug.clone())));
        out.insert("pane".into(), json!(item.is_some_and(|i| panes.contains(&i.slug))));
        out.insert("reviewItem".into(), json!(pr_items::review_item(&pr, &items).map(|i| i.slug.clone())));
        out.insert("worktree".into(), json!(path.as_deref().map(|p| short_path(Some(p)))));
        out.insert("served".into(), json!(path.as_deref().is_some_and(|p| crate::pf::serves(&status, p))));
        out.insert("jobKey".into(), json!(item.map(|i| i.slug.clone()).unwrap_or(label)));
        out.insert("prerequisites".into(), prerequisites.get(pr["url"].as_str().unwrap_or("")).cloned().unwrap_or_else(|| json!([])));
        Value::Object(out)
    }).collect();
    list["prs"] = Value::Array(prs);
    let mut out = json!({"ok": true, "prerequisitesRefreshing": crate::pr_deployment::refreshing()});
    for (k, v) in list.as_object().cloned().unwrap_or_default() {
        out[k] = v;
    }
    Some(out)
}
