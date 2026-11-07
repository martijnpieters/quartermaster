//! The review actions of `POST /api/action`: `review-preview` and `review-post` (a pending review on
//! GitHub filled from the item's GitHub drafts), `pr-thread-resolve`, `draft-place` and the move of a
//! draft to the review. Whether GitHub takes a line is `ReviewAnchors.refusal`'s say.

use serde_json::{json, Map, Value};

use crate::drafts::{update, Halt};
use crate::gui_actions::{find, ok, Stop};
use crate::pending_review::PendingReview;
use crate::review_anchors::E;
use crate::work_item::WorkItem;

type R<T> = Result<T, Stop>;

fn to_s(v: &Value) -> R<String> {
    match v {
        Value::Null => Ok(String::new()),
        Value::String(t) => Ok(t.clone()),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Number(n) if n.is_i64() || n.is_u64() => Ok(n.to_string()),
        _ => Err(Stop::Unported),
    }
}

fn from_e(e: E) -> Stop {
    match e {
        E::Arg(m) => Stop::refuse(m),
        E::Unsure => Stop::Unported,
    }
}

fn halt(h: Halt) -> Stop {
    match h {
        Halt::Refuse(m) => Stop::refuse(m),
        Halt::Unported => Stop::Unported,
    }
}

fn repo_root(item: &WorkItem) -> R<String> {
    let project = item.project();
    match crate::project_map::root_for(project.as_deref()).filter(|r| std::path::Path::new(&crate::data_root::expand_path(r)).is_dir()) {
        Some(root) => Ok(crate::data_root::expand_path(&root).to_string_lossy().into_owned()),
        None => Err(Stop::refuse(format!("no repo root configured for project '{}'", project.unwrap_or_default()))),
    }
}

/// `GuiActions#refuse_unanchorable`: the guard before a draft goes to the review.
pub(crate) fn refuse_unanchorable(item: &WorkItem, entry: &Map<String, Value>) -> R<()> {
    let root = repo_root(item)?;
    match crate::review_anchors::refusal_stored(&root, entry).map_err(from_e)? {
        Some(why) => Err(Stop::refuse(why)),
        None => Ok(()),
    }
}

/// The entry of a drafts file by id (`Drafts#find`).
fn find_draft(slug: &str, id: &str) -> R<Option<Map<String, Value>>> {
    let entries = crate::drafts::entries(slug).ok_or(Stop::Unported)?;
    Ok(entries.into_iter().find(|e| e.get("id") == Some(&json!(id))))
}

/// `draft-put` of a GitHub draft with a comment: the anchor check comes before the put.
pub(crate) fn guard_put(slug: &str, raw: &Map<String, Value>) -> R<()> {
    let item = find(slug)?;
    refuse_unanchorable(&item, raw)
}

/// `draft-move` to the review: a draft of the PR scope is checked like a put.
pub(crate) fn guard_move(slug: &str, id: &str) -> R<()> {
    let item = find(slug)?;
    let Some(mut entry) = find_draft(&item.slug, id)? else { return Err(Stop::refuse(format!("no draft {id} on {}", item.slug))) };
    if entry.get("number").is_some_and(|n| !matches!(n, Value::Null | Value::Bool(false))) {
        let subject = match entry.get("subject") {
            Some(v) if !matches!(v, Value::Null | Value::Bool(false)) => v.clone(),
            _ => json!("line"),
        };
        entry.insert("subject".into(), subject);
        refuse_unanchorable(&item, &entry)?;
    }
    Ok(())
}

/// `draft-place`: the outdated card puts a GitHub draft on a line of the head, the file, or its commit.
pub(crate) fn place(slug: &str, args: &Map<String, Value>) -> R<Value> {
    let item = find(slug)?;
    let id = to_s(args.get("id").unwrap_or(&Value::Null))?;
    if !crate::gui_actions::slug_shaped(&item.slug) {
        return Err(Stop::refuse(format!("'{}' is not a work item slug", item.slug)));
    }
    let Some(entry) = find_draft(&item.slug, &id)? else { return Err(Stop::refuse(format!("no draft {} on {}", to_s(args.get("id").unwrap_or(&Value::Null))?, item.slug))) };
    if entry.get("kind") != Some(&json!("github-comment")) {
        return Err(Stop::refuse("only a review draft has a place on GitHub"));
    }
    let arg = |k: &str| args.get(k).cloned().unwrap_or(Value::Null);
    let how = args.get("how").and_then(Value::as_str).unwrap_or("").to_string();
    let mut fields = Map::new();
    match (args.get("how"), how.as_str()) {
        (Some(Value::String(_)), "commit") => {
            fields.insert("pin".into(), json!("commit"));
        }
        (Some(Value::String(_)), "line") | (Some(Value::String(_)), "file") => {
            for (k, v) in [("commit", arg("commit")), ("base", arg("base")), ("range", Value::Null), ("pin", Value::Null), ("start_line", Value::Null), ("start_side", Value::Null)] {
                fields.insert(k.into(), v);
            }
            if how == "line" {
                for k in ["path", "side", "line", "code"] {
                    if let Some(v) = args.get(k) {
                        fields.insert(k.into(), v.clone());
                    }
                }
            }
            fields.insert("subject".into(), json!(how));
        }
        (other, _) => {
            let shown = match other {
                None | Some(Value::Null) => "nil".to_string(),
                Some(Value::String(t)) => crate::ruby_text::inspect_ascii(t).ok_or(Stop::Unported)?,
                Some(Value::Bool(b)) => b.to_string(),
                Some(Value::Number(n)) if n.is_i64() || n.is_u64() => n.to_string(),
                _ => return Err(Stop::Unported),
            };
            return Err(Stop::refuse(format!("a draft is placed on a line, the file or its commit, not {shown}")));
        }
    }
    if how != "commit" {
        let mut placed = entry.clone();
        for (k, v) in &fields {
            placed.insert(k.clone(), v.clone());
        }
        placed.retain(|_, v| !v.is_null());
        refuse_unanchorable(&item, &placed)?;
    }
    let stored = relocate_bump(&item.slug, &id, &fields).map_err(halt)?;
    let mut out = ok();
    out["draft"] = stored;
    Ok(out)
}

const RELOCATED: [&str; 11] = ["path", "side", "line", "code", "range", "start_line", "start_side", "base", "commit", "subject", "pin"];

/// `Drafts#relocate(id, fields, bump: true)`: the draft's new place, its comment unchanged.
fn relocate_bump(slug: &str, id: &str, fields: &Map<String, Value>) -> Result<Value, Halt> {
    update(slug, |list| {
        let Some(entry) = list.iter_mut().find(|e| e.get("id") == Some(&json!(id)) && e.get("kind") == Some(&json!("github-comment"))) else { return Ok(Value::Null) };
        let Some(entry) = entry.as_object_mut() else { return Err(Halt::Unported) };
        for k in RELOCATED {
            if let Some(v) = fields.get(k) {
                if v.is_null() {
                    entry.shift_remove(k);
                } else {
                    entry.insert(k.into(), v.clone());
                }
            }
        }
        entry.shift_remove("failed");
        let rev = match entry.get("rev") {
            None | Some(Value::Null) => 0,
            Some(Value::Number(n)) if n.is_i64() => n.as_i64().unwrap(),
            _ => return Err(Halt::Unported),
        };
        entry.insert("rev".into(), json!(rev + 1));
        entry.insert("updatedAt".into(), json!(crate::clock::now_utc_iso()));
        Ok(Value::Object(entry.clone()))
    })
}

/// The drafts of one PR the confirmation or the post acts on: `(review, entries)`.
fn review_drafts(item: &WorkItem, args: &Map<String, Value>) -> R<(PendingReview, Vec<Map<String, Value>>)> {
    let root = repo_root(item)?;
    let repo = args.get("repo").cloned().unwrap_or(Value::Null);
    let review = PendingReview::new(&root, &Value::String(to_s(&repo)?), args.get("number").unwrap_or(&Value::Null)).map_err(from_e)?;
    let all = crate::drafts::entries(&item.slug).ok_or(Stop::Unported)?;
    let mut entries: Vec<Map<String, Value>> = vec![];
    for e in all {
        let kind = e.get("kind").and_then(Value::as_str).unwrap_or("");
        if !(kind == "github-comment" || kind == "thread-reply") {
            continue;
        }
        let repo_matches = to_s(e.get("repo").unwrap_or(&Value::Null))?.to_lowercase() == review.repo.to_lowercase();
        let number_matches = match e.get("number") {
            Some(Value::Number(n)) => n.as_i64() == Some(review.number),
            _ => false,
        };
        if repo_matches && number_matches {
            entries.push(e);
        }
    }
    if let Some(Value::Array(ids)) = args.get("ids") {
        entries.retain(|e| ids.contains(e.get("id").unwrap_or(&Value::Null)));
    }
    if entries.is_empty() {
        return Err(Stop::refuse(format!("no review drafts for {}", review.label())));
    }
    Ok((review, entries))
}

pub(crate) fn preview(slug: &str, args: &Map<String, Value>) -> R<Value> {
    let item = find(slug)?;
    let (review, entries) = review_drafts(&item, args)?;
    let preview = review.preview(&entries).map_err(from_e)?;
    let mut out = ok();
    out["preview"] = preview;
    Ok(out)
}

/// `review-post`: the posting runs as the item's background job, a GitHub call per draft.
pub(crate) fn post(slug: &str, args: &Map<String, Value>) -> R<Value> {
    use crate::gui_jobs::Failure;
    let item = find(slug)?;
    let (review, entries) = review_drafts(&item, args)?;
    let (key, label, item_slug) = (item.slug.clone(), format!("posting to {}…", review.label()), item.slug.clone());
    let (root, repo, number) = (repo_root(&item)?, review.repo.clone(), review.number);
    drop(review);
    Ok(crate::gui_jobs::background(&key, &label, false, move || {
        let failure = |e: E| match e {
            E::Arg(m) => Failure::Argument(m),
            E::Unsure => Failure::Other("a draft holds a value this port can't post yet".into(), "NotImplementedError"),
        };
        let review = PendingReview::new(&root, &json!(repo), &json!(number)).map_err(failure)?;
        let result = review.post(&entries, &item_slug).map_err(failure)?;
        Ok(post_result(&review, &result))
    }))
}

fn post_result(review: &PendingReview, result: &Value) -> Value {
    let n = |k: &str| result[k].as_i64().unwrap_or(0);
    let failed: Vec<String> = result["failed"].as_array().map(|l| l.iter().map(|v| v.as_str().unwrap_or("").to_string()).collect()).unwrap_or_default();
    let mut parts = vec![format!("{} added to your pending review on {} — submit it on GitHub", n("posted"), review.label())];
    if n("outdated") > 0 {
        parts.push(format!("{} outdated, not posted", n("outdated")));
    }
    let mut out = json!({"ok": failed.is_empty(), "message": parts.join(" · "), "review": true});
    if !failed.is_empty() {
        out["error"] = json!(format!("{} · {} failed and stay in the tray: {}", parts.join(" · "), failed.len(), failed[0]));
    }
    if n("fallbacks") > 0 {
        out["warning"] = json!(format!("{} couldn't go on their old commit and became file comments quoting their line", n("fallbacks")));
    }
    out
}

/// `pr-thread-resolve`: resolve or unresolve a review thread on GitHub at once.
pub(crate) fn resolve(args: &Map<String, Value>) -> R<Value> {
    let get = |k: &str| args.get(k).unwrap_or(&Value::Null);
    let repo = match get("repo") {
        Value::String(r) => r.clone(),
        other => return Err(Stop::refuse(format!("'{}' is not an owner/name repo", to_s(other)?))),
    };
    let number = to_s(get("number"))?;
    let resolved = *get("resolve") != Value::Bool(false);
    match crate::review_threads::resolve(&repo, &number, get("thread"), resolved) {
        Ok(v) => {
            let mut out = ok();
            out["resolved"] = v;
            Ok(out)
        }
        Err(Some(m)) => Err(Stop::refuse(m)),
        Err(None) => Err(Stop::Unported),
    }
}

